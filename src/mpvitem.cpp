#include "mpvitem.h"
#include <QOpenGLContext>
#include <QOpenGLFramebufferObject>
#include <QOpenGLFunctions>
#include <QMetaObject>
#include <QPointer>
#include <QUrl>
#include <mpv/render.h>
#include <mpv/render_gl.h>

static void *getProc(void *, const char *name) {
    return reinterpret_cast<void *>(QOpenGLContext::currentContext()->getProcAddress(name));
}

class MpvRenderer : public QQuickFramebufferObject::Renderer {
public:
    MpvRenderer(std::shared_ptr<MpvHandle> handle, QPointer<MpvItem> item)
        : m_handle(std::move(handle)), m_item(item) {}

    QOpenGLFramebufferObject *createFramebufferObject(const QSize &size) override {
        if (m_context)
            return QQuickFramebufferObject::Renderer::createFramebufferObject(size);

        mpv_opengl_init_params gl{getProc, nullptr};
        const char *api = MPV_RENDER_API_TYPE_OPENGL;
        mpv_render_param params[] = {{MPV_RENDER_PARAM_API_TYPE, const_cast<char *>(api)},
                                     {MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, &gl},
                                     {MPV_RENDER_PARAM_INVALID, nullptr}};
        int result = mpv_render_context_create(&m_context, m_handle->ptr, params);
        if (result >= 0) {
            mpv_render_context_set_update_callback(m_context, &MpvRenderer::onUpdate, this);
            if (m_item) QMetaObject::invokeMethod(m_item, [item = m_item] { if (item) item->rendererReady(); }, Qt::QueuedConnection);
        }
        return QQuickFramebufferObject::Renderer::createFramebufferObject(size);
    }
    ~MpvRenderer() override {
        if (m_context) {
            mpv_render_context_set_update_callback(m_context, nullptr, nullptr);
            mpv_render_context_free(m_context);
        }
    }
    static void onUpdate(void *opaque) {
        auto *self = static_cast<MpvRenderer *>(opaque);
        if (self->m_item)
            QMetaObject::invokeMethod(self->m_item, [item = self->m_item] { if (item) item->update(); }, Qt::QueuedConnection);
    }
    void render() override {
        if (m_context) {
            mpv_render_context_update(m_context);
            auto *fbo = framebufferObject();
            mpv_opengl_fbo target{int(fbo->handle()), fbo->width(), fbo->height(), 0};
            int flip = 1;
            mpv_render_param params[] = {{MPV_RENDER_PARAM_OPENGL_FBO, &target},
                                         {MPV_RENDER_PARAM_FLIP_Y, &flip},
                                         {MPV_RENDER_PARAM_INVALID, nullptr}};
            mpv_render_context_render(m_context, params);
            QOpenGLContext::currentContext()->functions()->glFlush();
        }
    }
private:
    std::shared_ptr<MpvHandle> m_handle;
    QPointer<MpvItem> m_item;
    mpv_render_context *m_context = nullptr;
};

MpvItem::MpvItem(QQuickItem *parent) : QQuickFramebufferObject(parent), m_handle(std::make_shared<MpvHandle>()) {
    setMirrorVertically(true);
    m_handle->ptr = mpv_create();
    if (!m_handle->ptr) { m_error = "Could not create mpv"; return; }
    mpv_set_option_string(m_handle->ptr, "vo", "libmpv");
    // The QML item supplies an OpenGL render context below. Explicitly select
    // mpv's OpenGL GPU backend; otherwise recent mpv builds may choose Vulkan
    // before the render API is created and crash during context setup.
    mpv_set_option_string(m_handle->ptr, "gpu-api", "opengl");
    mpv_set_option_string(m_handle->ptr, "hwdec", "no");
    mpv_set_option_string(m_handle->ptr, "keep-open", "yes");
    if (mpv_initialize(m_handle->ptr) < 0) m_error = "Could not initialize mpv";
    connect(&m_timer, &QTimer::timeout, this, &MpvItem::poll);
    m_timer.start(150);
}
QQuickFramebufferObject::Renderer *MpvItem::createRenderer() const {
    return new MpvRenderer(m_handle, const_cast<MpvItem *>(this));
}
void MpvItem::rendererReady() {
    m_rendererReady = true;
    if (!m_pendingSource.isEmpty()) {
        QString pending = m_pendingSource;
        m_pendingSource.clear();
        open(pending);
    }
}
void MpvItem::open(const QString &source) {
    if (!m_handle->ptr) return;
    QString value = source.trimmed();
    if (value.isEmpty()) return;
    if (!m_rendererReady) { m_pendingSource = value; return; }
    QUrl url(value);
    if (url.isLocalFile()) value = url.toLocalFile();
    m_source = value;
    m_title = QUrl::fromUserInput(value).fileName();
    if (m_title.isEmpty()) m_title = value;
    m_error.clear(); m_position = 0; m_duration = 0;
    emit sourceChanged(); emit stateChanged();
    QByteArray utf8 = value.toUtf8();
    const char *args[] = {"loadfile", utf8.constData(), "replace", nullptr};
    int result = mpv_command_async(m_handle->ptr, 0, args);
    if (result < 0) { m_error = QString::fromUtf8(mpv_error_string(result)); emit stateChanged(); }
}
void MpvItem::togglePause() {
    if (!m_handle->ptr) return;
    int value = !m_paused;
    mpv_set_property(m_handle->ptr, "pause", MPV_FORMAT_FLAG, &value);
    poll();
}
void MpvItem::seek(double seconds) {
    if (!m_handle->ptr) return;
    QByteArray value = QByteArray::number(seconds);
    const char *args[] = {"seek", value.constData(), "relative", nullptr};
    mpv_command_async(m_handle->ptr, 0, args);
}
void MpvItem::seekTo(double seconds) {
    if (!m_handle->ptr) return;
    QByteArray value = QByteArray::number(seconds);
    const char *args[] = {"seek", value.constData(), "absolute", nullptr};
    mpv_command_async(m_handle->ptr, 0, args);
}
void MpvItem::toggleMute() {
    if (!m_handle->ptr) return;
    int value = !m_muted;
    mpv_set_property(m_handle->ptr, "mute", MPV_FORMAT_FLAG, &value);
    poll();
}
void MpvItem::setVolume(double volume) {
    if (!m_handle->ptr) return;
    volume = qBound(0.0, volume, 100.0);
    mpv_set_property(m_handle->ptr, "volume", MPV_FORMAT_DOUBLE, &volume);
    poll();
}
void MpvItem::poll() {
    if (!m_handle->ptr) return;
    while (mpv_event *event = mpv_wait_event(m_handle->ptr, 0)) {
        if (event->event_id == MPV_EVENT_NONE) break;
        if (event->event_id == MPV_EVENT_END_FILE) {
            auto *end = static_cast<mpv_event_end_file *>(event->data);
            if (end->reason == MPV_END_FILE_REASON_ERROR)
                m_error = QStringLiteral("Unable to play source: %1").arg(QString::fromUtf8(mpv_error_string(end->error)));
        }
        if (event->event_id == MPV_EVENT_FILE_LOADED) m_error.clear();
    }
    double d;
    int flag;
    if (mpv_get_property(m_handle->ptr, "time-pos", MPV_FORMAT_DOUBLE, &d) >= 0) m_position = d;
    if (mpv_get_property(m_handle->ptr, "duration", MPV_FORMAT_DOUBLE, &d) >= 0) m_duration = d;
    if (mpv_get_property(m_handle->ptr, "volume", MPV_FORMAT_DOUBLE, &d) >= 0) m_volume = d;
    if (mpv_get_property(m_handle->ptr, "pause", MPV_FORMAT_FLAG, &flag) >= 0) m_paused = flag;
    if (mpv_get_property(m_handle->ptr, "mute", MPV_FORMAT_FLAG, &flag) >= 0) m_muted = flag;
    char *title = nullptr;
    if (mpv_get_property(m_handle->ptr, "media-title", MPV_FORMAT_STRING, &title) >= 0 && title) {
        m_title = QString::fromUtf8(title);
        mpv_free(title);
    }
    emit stateChanged();
}

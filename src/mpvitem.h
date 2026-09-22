#pragma once
#include <QQuickFramebufferObject>
#include <QTimer>
#include <QString>
#include <memory>
#include <mpv/client.h>

struct MpvHandle {
    mpv_handle *ptr = nullptr;
    ~MpvHandle() { if (ptr) mpv_terminate_destroy(ptr); }
};

class MpvItem : public QQuickFramebufferObject {
    Q_OBJECT
    Q_PROPERTY(QString source READ source NOTIFY sourceChanged)
    Q_PROPERTY(QString title READ title NOTIFY stateChanged)
    Q_PROPERTY(QString error READ error NOTIFY stateChanged)
    Q_PROPERTY(double position READ position NOTIFY stateChanged)
    Q_PROPERTY(double duration READ duration NOTIFY stateChanged)
    Q_PROPERTY(double volume READ volume WRITE setVolume NOTIFY stateChanged)
    Q_PROPERTY(bool paused READ paused NOTIFY stateChanged)
    Q_PROPERTY(bool muted READ muted NOTIFY stateChanged)
public:
    explicit MpvItem(QQuickItem *parent = nullptr);
    Renderer *createRenderer() const override;
    void rendererReady();
    QString source() const { return m_source; }
    QString title() const { return m_title; }
    QString error() const { return m_error; }
    double position() const { return m_position; }
    double duration() const { return m_duration; }
    double volume() const { return m_volume; }
    bool paused() const { return m_paused; }
    bool muted() const { return m_muted; }
    Q_INVOKABLE void open(const QString &source);
    Q_INVOKABLE void togglePause();
    Q_INVOKABLE void seek(double seconds);
    Q_INVOKABLE void seekTo(double seconds);
    Q_INVOKABLE void toggleMute();
    void setVolume(double volume);
    std::shared_ptr<MpvHandle> handle() const { return m_handle; }
signals:
    void sourceChanged();
    void stateChanged();
private:
    void poll();
    std::shared_ptr<MpvHandle> m_handle;
    QTimer m_timer;
    QString m_source, m_title, m_error, m_pendingSource;
    bool m_rendererReady = false;
    double m_position = 0, m_duration = 0, m_volume = 100;
    bool m_paused = false, m_muted = false;
};

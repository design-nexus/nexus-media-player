#include "theme.h"
#include <QDir>
#include <QFile>
#include <QFontDatabase>
#include <QProcess>
#include <QRegularExpression>
#include <QStandardPaths>

Theme::Theme(QObject *parent) : QObject(parent) {
    reload();
    connect(&m_timer, &QTimer::timeout, this, &Theme::reload);
    m_timer.start(1000);
}
void Theme::reload() {
    const QString path = QDir::homePath() + "/.local/state/omarchy/current/theme/colors.toml";
    QFile file(path);
    QHash<QString, QColor> colors;
    if (file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        const QRegularExpression line(R"re(^\s*([a-z_]+)\s*=\s*"(#[0-9a-fA-F]{6})")re");
        while (!file.atEnd()) {
            auto match = line.match(QString::fromUtf8(file.readLine()));
            if (match.hasMatch()) colors.insert(match.captured(1), QColor(match.captured(2)));
        }
    }
    bool change = false;
    auto update = [&](QColor &target, const QString &key) {
        if (colors.contains(key) && target != colors.value(key)) { target = colors.value(key); change = true; }
    };
    update(m_background, "background"); update(m_surface, "lighter_background");
    update(m_foreground, "foreground"); update(m_muted, "dark_foreground");
    update(m_accent, "accent"); update(m_red, "red");
    QProcess fc;
    fc.start("fc-match", {"monospace", "-f", "%{family}\\n"});
    if (fc.waitForFinished(500)) {
        QString family = QString::fromUtf8(fc.readAllStandardOutput()).split('\n').first().split(',').first().trimmed();
        if (!family.isEmpty() && family != m_font) { m_font = family; change = true; }
    }
    if (change) emit changed();
}

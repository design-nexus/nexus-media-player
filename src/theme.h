#pragma once
#include <QObject>
#include <QColor>
#include <QTimer>
#include <QDateTime>

class Theme : public QObject {
    Q_OBJECT
    Q_PROPERTY(QColor background READ background NOTIFY changed)
    Q_PROPERTY(QColor surface READ surface NOTIFY changed)
    Q_PROPERTY(QColor foreground READ foreground NOTIFY changed)
    Q_PROPERTY(QColor muted READ muted NOTIFY changed)
    Q_PROPERTY(QColor accent READ accent NOTIFY changed)
    Q_PROPERTY(QColor red READ red NOTIFY changed)
    Q_PROPERTY(QString fontFamily READ fontFamily NOTIFY changed)
public:
    explicit Theme(QObject *parent = nullptr);
    QColor background() const { return m_background; }
    QColor surface() const { return m_surface; }
    QColor foreground() const { return m_foreground; }
    QColor muted() const { return m_muted; }
    QColor accent() const { return m_accent; }
    QColor red() const { return m_red; }
    QString fontFamily() const { return m_font; }
signals:
    void changed();
private:
    void reload();
    QTimer m_timer;
    QColor m_background{"#1e1e2e"}, m_surface{"#313244"}, m_foreground{"#cdd6f4"};
    QColor m_muted{"#6c7086"}, m_accent{"#89b4fa"}, m_red{"#f38ba8"};
    QString m_font{"monospace"};
};

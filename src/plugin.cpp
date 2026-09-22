#include <QQmlExtensionPlugin>
#include <qqml.h>
#include "mpvitem.h"
#include "theme.h"

class NexusMpvPlugin : public QQmlExtensionPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID QQmlExtensionInterface_iid)
public:
    void registerTypes(const char *uri) override {
        qmlRegisterType<MpvItem>(uri, 1, 0, "MpvItem");
        qmlRegisterType<Theme>(uri, 1, 0, "OmarchyTheme");
    }
};
#include "plugin.moc"

import QtQuick
Rectangle {
    id: button
    property alias text: label.text
    property var colors
    property int minWidth: 28
    property int fontSize: 11
    signal clicked()
    width: Math.max(minWidth, label.implicitWidth + 12)
    height: 27
    radius: 3
    color: mouse.containsMouse ? colors.accent : colors.surface
    Text {
        id: label
        anchors.centerIn: parent
        color: mouse.containsMouse ? button.colors.background : button.colors.foreground
        font.family: button.colors.fontFamily
        font.bold: true
        font.pixelSize: button.fontSize
    }
    MouseArea { id: mouse; anchors.fill: parent; hoverEnabled: true; onClicked: button.clicked() }
}

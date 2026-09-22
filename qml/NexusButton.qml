import QtQuick
Rectangle {
    id: button
    property alias text: label.text
    property var palette
    signal clicked()
    width: Math.max(46, label.implicitWidth + 20)
    height: 30
    radius: 4
    color: mouse.containsMouse ? palette.accent : button.palette.surface
    Text {
        id: label
        anchors.centerIn: parent
        color: mouse.containsMouse ? palette.background : button.palette.foreground
        font.family: button.palette.fontFamily
        font.bold: true
        font.pixelSize: 10
    }
    MouseArea { id: mouse; anchors.fill: parent; hoverEnabled: true; onClicked: button.clicked() }
}

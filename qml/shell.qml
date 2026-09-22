import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import Quickshell
import Nexus.Mpv 1.0

FloatingWindow {
    id: win
    visible: true
    implicitWidth: 960
    implicitHeight: 610
    title: player.title ? player.title + " · Nexus Media Player" : "Nexus Media Player"
    color: theme.background

    OmarchyTheme { id: theme }
    FileDialog {
        id: picker
        title: "Open media"
        onAccepted: player.open(selectedFile.toString())
    }
    function formatTime(seconds) {
        if (!isFinite(seconds) || seconds < 0) return "00:00"
        let n = Math.floor(seconds)
        let h = Math.floor(n / 3600)
        let m = Math.floor((n % 3600) / 60)
        let s = n % 60
        return (h ? h.toString().padStart(2, "0") + ":" : "") + m.toString().padStart(2, "0") + ":" + s.toString().padStart(2, "0")
    }
    property real scrollRemainder: 0
    function horizontalSeek(wheel) {
        scrollRemainder += wheel.pixelDelta.x !== 0 ? wheel.pixelDelta.x / 50 : wheel.angleDelta.x / 120
        while (scrollRemainder >= 1) { player.seek(5); scrollRemainder -= 1 }
        while (scrollRemainder <= -1) { player.seek(-5); scrollRemainder += 1 }
    }
    Component.onCompleted: {
        if (Quickshell.env("NEXUS_SOURCE")) player.open(Quickshell.env("NEXUS_SOURCE"))
        keyboard.forceActiveFocus()
    }

    Item {
        id: keyboard
        anchors.fill: parent
        focus: true
        Shortcut { sequence: "Left"; enabled: !sourceInput.activeFocus; onActivated: player.seek(-5) }
        Shortcut { sequence: "Right"; enabled: !sourceInput.activeFocus; onActivated: player.seek(5) }
        Shortcut { sequence: "Space"; enabled: !sourceInput.activeFocus; onActivated: player.togglePause() }
        Shortcut { sequence: "F"; enabled: !sourceInput.activeFocus; onActivated: win.fullscreen = !win.fullscreen }
        Shortcut { sequence: "M"; enabled: !sourceInput.activeFocus; onActivated: player.toggleMute() }
        Shortcut { sequence: "Escape"; enabled: win.fullscreen; onActivated: win.fullscreen = false }
        DropArea {
            anchors.fill: parent
            onDropped: drop => {
                if (drop.urls.length) player.open(drop.urls[0].toString())
            }
        }
        Column {
            anchors.fill: parent
            spacing: 0
            Rectangle {
                width: parent.width
                height: win.fullscreen ? 0 : 46
                visible: !win.fullscreen
                color: theme.background
                Row {
                    anchors.fill: parent
                    anchors.leftMargin: 16
                    anchors.rightMargin: 16
                    spacing: 10
                    Text { text: "◈"; color: theme.accent; font.family: theme.fontFamily; font.pixelSize: 18; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: "NEXUS"; color: theme.foreground; font.family: theme.fontFamily; font.bold: true; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter }
                    Rectangle { width: 1; height: 18; color: theme.surface; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: player.title || "MEDIA PLAYER"; color: theme.muted; elide: Text.ElideMiddle; width: Math.max(120, win.width - 350); font.family: theme.fontFamily; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter }
                    NexusButton { palette: theme; text: "OPEN"; anchors.verticalCenter: parent.verticalCenter; onClicked: picker.open() }
                    NexusButton { palette: theme; text: win.fullscreen ? "WINDOW" : "FULL"; anchors.verticalCenter: parent.verticalCenter; onClicked: win.fullscreen = !win.fullscreen }
                }
            }
            Rectangle { width: parent.width; height: 1; color: theme.surface; visible: !win.fullscreen }
            Item {
                id: stage
                width: parent.width
                height: parent.height - (win.fullscreen ? 60 : 159)
                Rectangle { anchors.fill: parent; color: "#09090e" }
                MpvItem { id: player; anchors.fill: parent }
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton
                    onClicked: { player.togglePause(); keyboard.forceActiveFocus() }
                    onWheel: wheel => { win.horizontalSeek(wheel); wheel.accepted = true }
                }
                Column {
                    anchors.centerIn: parent
                    spacing: 14
                    visible: player.source === ""
                    Text { text: "◈"; color: theme.accent; font.family: theme.fontFamily; font.pixelSize: 42; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: "DROP MEDIA HERE"; color: theme.foreground; font.family: theme.fontFamily; font.pixelSize: 15; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: "OPEN A FILE OR PASTE A URL BELOW"; color: theme.muted; font.family: theme.fontFamily; font.pixelSize: 10; anchors.horizontalCenter: parent.horizontalCenter }
                }
                Rectangle {
                    visible: player.error.length > 0
                    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
                    anchors.margins: 16; height: 34; color: theme.surface
                    Text { anchors.centerIn: parent; text: player.error; color: theme.red; font.family: theme.fontFamily; font.pixelSize: 11 }
                }
            }
            Rectangle { width: parent.width; height: 1; color: theme.surface }
            Rectangle {
                width: parent.width; height: 58; color: theme.background
                Row {
                    anchors.fill: parent; anchors.margins: 14; spacing: 12
                    NexusButton { palette: theme; text: player.paused ? "▶" : "Ⅱ"; onClicked: player.togglePause() }
                    Text { text: win.formatTime(player.position); color: theme.foreground; font.family: theme.fontFamily; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter }
                    Rectangle {
                        id: seekTrack
                        width: Math.max(80, win.width - 375); height: 6; radius: 3; color: theme.surface
                        anchors.verticalCenter: parent.verticalCenter
                        Rectangle { width: parent.width * Math.min(1, player.duration > 0 ? player.position / player.duration : 0); height: parent.height; radius: 3; color: theme.accent }
                        MouseArea {
                            anchors.fill: parent
                            onPressed: mouse => { if (player.duration > 0) player.seekTo(mouse.x / width * player.duration); keyboard.forceActiveFocus() }
                            onPositionChanged: mouse => { if (pressed && player.duration > 0) player.seekTo(Math.max(0, Math.min(1, mouse.x / width)) * player.duration) }
                            onWheel: wheel => { win.horizontalSeek(wheel); wheel.accepted = true }
                        }
                    }
                    Text { text: win.formatTime(player.duration); color: theme.muted; font.family: theme.fontFamily; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter }
                    NexusButton { palette: theme; text: player.muted ? "MUTED" : "VOL"; onClicked: player.toggleMute() }
                    Rectangle {
                        width: 76; height: 6; radius: 3; color: theme.surface; anchors.verticalCenter: parent.verticalCenter
                        Rectangle { width: parent.width * player.volume / 100; height: parent.height; radius: 3; color: theme.accent }
                        MouseArea {
                            anchors.fill: parent
                            onPressed: mouse => player.volume = Math.max(0, Math.min(100, mouse.x / width * 100))
                            onPositionChanged: mouse => { if (pressed) player.volume = Math.max(0, Math.min(100, mouse.x / width * 100)) }
                        }
                    }
                }
            }
            Rectangle {
                visible: !win.fullscreen
                width: parent.width; height: win.fullscreen ? 0 : 54; color: theme.background
                Row {
                    anchors.fill: parent; anchors.leftMargin: 16; anchors.rightMargin: 16; anchors.bottomMargin: 12; spacing: 8
                    Rectangle {
                        width: win.width - 105; height: 32; radius: 4; color: theme.surface; anchors.verticalCenter: parent.verticalCenter
                        TextField {
                            id: sourceInput; anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 8
                            placeholderText: "PASTE FILE PATH OR URL"; color: theme.foreground
                            font.family: theme.fontFamily; font.pixelSize: 11
                            background: null
                            onAccepted: { player.open(text); keyboard.forceActiveFocus() }
                        }
                    }
                    NexusButton { palette: theme; text: "PLAY"; anchors.verticalCenter: parent.verticalCenter; onClicked: { player.open(sourceInput.text); keyboard.forceActiveFocus() } }
                }
            }
        }
    }
}

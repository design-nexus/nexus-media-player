import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import Quickshell
import Nexus.Mpv 1.0

FloatingWindow {
    id: win
    visible: true
    implicitWidth: 960
    implicitHeight: 610
    title: player.title ? player.title + " · Nexus Media Player" : "Nexus Media Player"
    color: theme.background

    property bool menuOpen: false
    property bool urlOpen: false
    property bool volumeOpen: false
    property bool uiVisible: true
    property bool chromeShown: !fullscreen || uiVisible
    property real scrollRemainder: 0

    OmarchyTheme { id: theme }
    FileDialog {
        id: picker
        title: "Open media"
        onAccepted: player.open(selectedFile.toString())
    }
    Timer {
        id: hideTimer
        interval: 3000
        onTriggered: {
            if (win.fullscreen && !win.menuOpen && !win.urlOpen && !win.volumeOpen)
                win.uiVisible = false
        }
    }
    function revealUi() {
        uiVisible = true
        if (fullscreen && !menuOpen && !urlOpen && !volumeOpen) hideTimer.restart()
    }
    function formatTime(seconds) {
        if (!isFinite(seconds) || seconds < 0) return "00:00"
        let n = Math.floor(seconds)
        let h = Math.floor(n / 3600)
        let m = Math.floor((n % 3600) / 60)
        let s = n % 60
        return (h ? h.toString().padStart(2, "0") + ":" : "") + m.toString().padStart(2, "0") + ":" + s.toString().padStart(2, "0")
    }
    function horizontalSeek(wheel) {
        scrollRemainder += wheel.pixelDelta.x !== 0 ? wheel.pixelDelta.x / 50 : wheel.angleDelta.x / 120
        while (scrollRemainder >= 1) { player.seek(5); scrollRemainder -= 1 }
        while (scrollRemainder <= -1) { player.seek(-5); scrollRemainder += 1 }
    }
    onFullscreenChanged: revealUi()
    onMenuOpenChanged: menuOpen ? hideTimer.stop() : revealUi()
    onUrlOpenChanged: urlOpen ? hideTimer.stop() : revealUi()
    onVolumeOpenChanged: volumeOpen ? hideTimer.stop() : revealUi()
    Component.onCompleted: {
        if (Quickshell.env("NEXUS_SOURCE")) player.open(Quickshell.env("NEXUS_SOURCE"))
        keyboard.forceActiveFocus()
    }

    Item {
        id: keyboard
        anchors.fill: parent
        focus: true
        Keys.onPressed: win.revealUi()
        Shortcut { sequence: "Left"; enabled: !urlInput.activeFocus; onActivated: { player.seek(-5); win.revealUi() } }
        Shortcut { sequence: "Right"; enabled: !urlInput.activeFocus; onActivated: { player.seek(5); win.revealUi() } }
        Shortcut { sequence: "Space"; enabled: !urlInput.activeFocus; onActivated: { player.togglePause(); win.revealUi() } }
        Shortcut { sequence: "F"; enabled: !urlInput.activeFocus; onActivated: win.fullscreen = !win.fullscreen }
        Shortcut { sequence: "M"; enabled: !urlInput.activeFocus; onActivated: { player.toggleMute(); win.revealUi() } }
        Shortcut { sequence: "Ctrl+O"; onActivated: { win.menuOpen = false; picker.open(); win.revealUi() } }
        Shortcut {
            sequence: "Escape"
            enabled: win.urlOpen || win.menuOpen || win.volumeOpen || win.fullscreen
            onActivated: {
                if (win.urlOpen) { win.urlOpen = false; keyboard.forceActiveFocus() }
                else if (win.menuOpen) win.menuOpen = false
                else if (win.volumeOpen) win.volumeOpen = false
                else win.fullscreen = false
            }
        }
        DropArea {
            anchors.fill: parent
            onDropped: drop => {
                if (drop.urls.length) player.open(drop.urls[0].toString())
                win.menuOpen = false
                win.urlOpen = false
                win.revealUi()
            }
        }
        Column {
            anchors.fill: parent
            spacing: 0
            Rectangle {
                id: topBar
                width: parent.width
                height: win.chromeShown ? 38 : 0
                visible: win.chromeShown
                color: theme.background
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 10
                    anchors.rightMargin: 8
                    spacing: 7
                    Text { text: "◈"; color: theme.accent; font.family: theme.fontFamily; font.pixelSize: 16; Layout.alignment: Qt.AlignVCenter }
                    Text { text: "NEXUS"; color: theme.foreground; font.family: theme.fontFamily; font.bold: true; font.pixelSize: 11; Layout.alignment: Qt.AlignVCenter }
                    Text {
                        text: player.title || "MEDIA PLAYER"
                        color: theme.muted
                        elide: Text.ElideMiddle
                        font.family: theme.fontFamily
                        font.pixelSize: 10
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                    }
                    NexusButton {
                        colors: theme; text: win.fullscreen ? "\uf066" : "\uf065"
                        fontSize: 14; minWidth: 28
                        Layout.alignment: Qt.AlignVCenter
                        onClicked: { win.fullscreen = !win.fullscreen; win.menuOpen = false }
                    }
                    NexusButton {
                        colors: theme; text: "\uf0c9"
                        fontSize: 14; minWidth: 28
                        Layout.alignment: Qt.AlignVCenter
                        onClicked: { win.menuOpen = !win.menuOpen; win.volumeOpen = false; win.revealUi() }
                    }
                }
            }
            Item {
                id: stage
                width: parent.width
                height: parent.height - topBar.height - controls.height
                Rectangle { anchors.fill: parent; color: "#09090e" }
                MpvItem { id: player; anchors.fill: parent }
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton
                    onPositionChanged: win.revealUi()
                    onClicked: {
                        if (win.menuOpen || win.urlOpen || win.volumeOpen) {
                            win.menuOpen = false; win.urlOpen = false; win.volumeOpen = false
                        } else player.togglePause()
                        keyboard.forceActiveFocus()
                        win.revealUi()
                    }
                    onWheel: wheel => { win.horizontalSeek(wheel); win.revealUi(); wheel.accepted = true }
                }
                Column {
                    anchors.centerIn: parent
                    spacing: 8
                    visible: player.source === ""
                    Text { text: "◈"; color: theme.accent; font.family: theme.fontFamily; font.pixelSize: 36; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: "DROP MEDIA HERE"; color: theme.foreground; font.family: theme.fontFamily; font.pixelSize: 13; anchors.horizontalCenter: parent.horizontalCenter }
                    Text { text: "USE THE MENU TO OPEN A FILE OR URL"; color: theme.muted; font.family: theme.fontFamily; font.pixelSize: 10; anchors.horizontalCenter: parent.horizontalCenter }
                }
                Rectangle {
                    visible: player.error.length > 0
                    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
                    anchors.margins: 8; height: 30; color: theme.surface
                    Text { anchors.centerIn: parent; text: player.error; color: theme.red; font.family: theme.fontFamily; font.pixelSize: 10 }
                }
            }
            Rectangle {
                id: controls
                width: parent.width
                height: win.chromeShown ? 43 : 0
                visible: win.chromeShown
                color: theme.background
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 8
                    NexusButton {
                        colors: theme; text: player.paused ? "▶" : "Ⅱ"
                        minWidth: 28; fontSize: 12
                        Layout.alignment: Qt.AlignVCenter
                        onClicked: { player.togglePause(); win.revealUi() }
                    }
                    Text { text: win.formatTime(player.position); color: theme.foreground; font.family: theme.fontFamily; font.pixelSize: 10; Layout.alignment: Qt.AlignVCenter }
                    Rectangle {
                        id: seekTrack
                        height: 5; radius: 2; color: theme.surface
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        Rectangle { width: parent.width * Math.min(1, player.duration > 0 ? player.position / player.duration : 0); height: parent.height; radius: 2; color: theme.accent }
                        MouseArea {
                            anchors.fill: parent
                            onPressed: mouse => { if (player.duration > 0) player.seekTo(mouse.x / width * player.duration); keyboard.forceActiveFocus(); win.revealUi() }
                            onPositionChanged: mouse => { if (pressed && player.duration > 0) player.seekTo(Math.max(0, Math.min(1, mouse.x / width)) * player.duration); win.revealUi() }
                            onWheel: wheel => { win.horizontalSeek(wheel); win.revealUi(); wheel.accepted = true }
                        }
                    }
                    Text { text: win.formatTime(player.duration); color: theme.muted; font.family: theme.fontFamily; font.pixelSize: 10; Layout.alignment: Qt.AlignVCenter }
                    NexusButton {
                        colors: theme; text: player.muted ? "\uf6a9" : "\uf028"
                        minWidth: 28; fontSize: 14
                        Layout.alignment: Qt.AlignVCenter
                        onClicked: { win.volumeOpen = !win.volumeOpen; win.menuOpen = false; win.revealUi() }
                    }
                }
            }
        }
        Rectangle {
            id: menuPopup
            z: 10
            visible: win.menuOpen && win.chromeShown
            width: 140; height: 62; radius: 4
            color: theme.surface
            anchors.top: parent.top; anchors.topMargin: topBar.height + 2
            anchors.right: parent.right; anchors.rightMargin: 8
            Column {
                anchors.fill: parent; anchors.margins: 3; spacing: 0
                Rectangle {
                    width: parent.width; height: 28; color: openFileMouse.containsMouse ? theme.accent : "transparent"; radius: 3
                    Text { text: "OPEN FILE"; anchors.left: parent.left; anchors.leftMargin: 8; anchors.verticalCenter: parent.verticalCenter; color: openFileMouse.containsMouse ? theme.background : theme.foreground; font.family: theme.fontFamily; font.pixelSize: 10 }
                    MouseArea { id: openFileMouse; anchors.fill: parent; hoverEnabled: true; onClicked: { win.menuOpen = false; picker.open() } }
                }
                Rectangle {
                    width: parent.width; height: 28; color: openUrlMouse.containsMouse ? theme.accent : "transparent"; radius: 3
                    Text { text: "OPEN URL / PATH"; anchors.left: parent.left; anchors.leftMargin: 8; anchors.verticalCenter: parent.verticalCenter; color: openUrlMouse.containsMouse ? theme.background : theme.foreground; font.family: theme.fontFamily; font.pixelSize: 10 }
                    MouseArea { id: openUrlMouse; anchors.fill: parent; hoverEnabled: true; onClicked: { win.menuOpen = false; win.urlOpen = true; urlInput.forceActiveFocus() } }
                }
            }
        }
        Rectangle {
            id: urlPopup
            z: 10
            visible: win.urlOpen
            width: Math.min(500, keyboard.width - 16)
            height: 36; radius: 4
            color: theme.surface
            anchors.top: parent.top; anchors.topMargin: topBar.height + 4
            anchors.horizontalCenter: parent.horizontalCenter
            TextField {
                id: urlInput
                anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 8
                placeholderText: "FILE PATH OR URL  ·  ENTER TO OPEN"
                color: theme.foreground
                font.family: theme.fontFamily; font.pixelSize: 10
                background: null
                onAccepted: {
                    if (text.trim().length) player.open(text)
                    win.urlOpen = false
                    keyboard.forceActiveFocus()
                }
            }
        }
        Rectangle {
            id: volumePopup
            z: 10
            visible: win.volumeOpen && win.chromeShown
            width: 38; height: 142; radius: 4
            color: theme.surface
            anchors.right: parent.right; anchors.rightMargin: 8
            anchors.bottom: parent.bottom; anchors.bottomMargin: controls.height + 3
            Rectangle {
                id: volumeTrack
                width: 6; height: 88; radius: 3
                anchors.top: parent.top; anchors.topMargin: 12
                anchors.horizontalCenter: parent.horizontalCenter
                color: theme.muted
                Rectangle { width: parent.width; height: parent.height * player.volume / 100; radius: 3; color: theme.accent; anchors.bottom: parent.bottom }
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -10
                    onPressed: mouse => player.volume = Math.max(0, Math.min(100, (1 - (mouse.y - 10) / volumeTrack.height) * 100))
                    onPositionChanged: mouse => { if (pressed) player.volume = Math.max(0, Math.min(100, (1 - (mouse.y - 10) / volumeTrack.height) * 100)) }
                }
            }
            NexusButton {
                colors: theme; text: player.muted ? "\uf6a9" : "\uf028"
                minWidth: 30; fontSize: 13
                anchors.bottom: parent.bottom; anchors.bottomMargin: 4
                anchors.horizontalCenter: parent.horizontalCenter
                onClicked: player.toggleMute()
            }
        }
    }
}

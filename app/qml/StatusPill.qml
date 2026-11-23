import QtQuick
import Zenith

// What the recorder is doing, at the top of the window.
Rectangle {
    id: pill
    property var status: ({ running: false, state: "stopped" })

    // The answer comes back by itself: a recorder that is slow to reply
    // does not hold the window up.
    function refresh() { Recorder.poll() }
    Connections {
        target: Recorder
        function onStatusReady(status) { pill.status = JSON.parse(status) }
    }
    Component.onCompleted: refresh()
    Timer { interval: 2000; running: pill.visible; repeat: true; onTriggered: pill.refresh() }

    readonly property bool capturing: status.state === "capturing" || status.state === "saving"
    readonly property string words: !status.running ? "Recorder is not running"
        : capturing ? (status.game ? "Capturing " + status.game : "Capturing") + "  ·  " + status.bufferSecs + " s"
        : status.state === "failed" ? "Capture failed"
        : status.state === "restarting" ? "Restarting capture"
        : status.forced === false ? "Capture is off"
        : "Waiting for a game"

    implicitWidth: row.implicitWidth + 24
    implicitHeight: 30
    radius: Theme.radius
    color: Theme.surface
    border.color: Theme.line

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 8
        Rectangle {
            width: 8
            height: 8
            radius: 4
            anchors.verticalCenter: parent.verticalCenter
            color: pill.capturing ? (pill.status.fallback ? Theme.warn : Theme.record) : !pill.status.running || pill.status.state === "failed" ? Theme.warn : Theme.textFaint
            SequentialAnimation on opacity {
                running: pill.capturing
                loops: Animation.Infinite
                alwaysRunToEnd: true
                NumberAnimation { to: 0.35; duration: 900; easing.type: Easing.InOutSine }
                NumberAnimation { to: 1; duration: 900; easing.type: Easing.InOutSine }
            }
        }
        Text {
            text: pill.words
            color: Theme.textDim
            font.family: Theme.monoFamily
            font.pixelSize: 11
            font.letterSpacing: 1.2
            font.capitalization: Font.AllUppercase
            anchors.verticalCenter: parent.verticalCenter
        }
    }
    // What there is to know beyond the words: why it failed, or how it is
    // recording, and that the way it found is the slower one.
    readonly property string more: status.detail ? status.detail
        : status.recording ? (status.fallback ? "The fast way did not work here, so this is the slower one: " : "Recording with ") + status.recording
        : ""
    Tip { shown: hover.hovered && pill.more !== ""; text: pill.more }
    HoverHandler { id: hover }
}

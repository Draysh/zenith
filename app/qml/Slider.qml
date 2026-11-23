import QtQuick

// A horizontal slider from `from` to `to`.
Item {
    id: slider
    property real value: 0
    property real from: 0
    property real to: 1
    property color fill: Theme.action
    signal moved(real value)
    signal pressed()
    signal released()
    // How much one notch of the wheel moves it, for fine tuning; at 0 the
    // wheel is left to whatever scrolls under the slider.
    property real wheelStep: 0
    // The step being made is a fine one (the wheel's): what listens does
    // not round it to a resting place.
    property bool fine: false

    // Reachable with Tab; the arrow keys move it a twentieth at a time.
    activeFocusOnTab: true
    function nudge(steps) {
        slider.pressed()
        slider.moved(Math.max(from, Math.min(to, value + steps * (to - from) / 20)))
        slider.released()
    }
    Keys.onLeftPressed: nudge(-1)
    Keys.onRightPressed: nudge(1)
    Accessible.role: Accessible.Slider
    Accessible.onIncreaseAction: nudge(1)
    Accessible.onDecreaseAction: nudge(-1)

    implicitWidth: 120
    implicitHeight: 20
    readonly property real fraction: Math.max(0, Math.min(1, (value - from) / (to - from)))

    Rectangle {
        id: groove
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width
        height: 4
        radius: 2
        color: Theme.line
        Rectangle { width: parent.width * slider.fraction; height: parent.height; radius: 2; color: slider.fill }
    }
    Rectangle {
        x: slider.fraction * (slider.width - width)
        anchors.verticalCenter: parent.verticalCenter
        // A bar across the groove, like the mark on a fader.
        width: 4
        height: area.pressed || area.containsMouse ? 18 : 14
        radius: 1
        color: Theme.text
        Behavior on height { NumberAnimation { duration: Theme.quick } }
    }
    MouseArea {
        id: area
        anchors.fill: parent
        anchors.margins: -6
        hoverEnabled: true
        function at(x) {
            const f = Math.max(0, Math.min(1, (x - 6) / slider.width))
            slider.moved(slider.from + f * (slider.to - slider.from))
        }
        onPressed: mouse => {
            // Taken hold of while the wheel's turn is still open: one hold.
            if (wheelEnd.running) { wheelEnd.stop() } else { slider.pressed() }
            at(mouse.x)
        }
        // A turn of the wheel is one hold from its first notch until it
        // rests, so it is one step to undo however many notches it was.
        onWheel: wheel => {
            const turn = wheel.angleDelta.y !== 0 ? wheel.angleDelta.y : -wheel.angleDelta.x
            if (slider.wheelStep <= 0 || turn === 0) { wheel.accepted = false; return }
            if (pressed) { return }
            if (!wheelEnd.running) { slider.pressed() }
            wheelEnd.restart()
            const next = Math.round((slider.value + (turn > 0 ? 1 : -1) * slider.wheelStep) / slider.wheelStep) * slider.wheelStep
            slider.fine = true
            slider.moved(Math.max(slider.from, Math.min(slider.to, next)))
            slider.fine = false
        }
        onPositionChanged: mouse => { if (pressed) { at(mouse.x) } }
        onReleased: slider.released()
        // The press was taken away (the window lost the pointer): it still ends.
        onCanceled: slider.released()
    }
    Timer { id: wheelEnd; interval: 350; onTriggered: slider.released() }
    Rectangle {
        anchors.fill: parent
        anchors.margins: -4
        radius: Theme.radius
        color: "transparent"
        border.width: 2
        border.color: Theme.action
        visible: slider.activeFocus
    }
}

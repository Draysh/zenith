import QtQuick

// An on/off switch.
Rectangle {
    id: toggle
    property bool on: false
    signal toggled(bool on)

    // Reachable with Tab; Space or Enter presses it.
    activeFocusOnTab: true
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            toggle.toggled(!toggle.on)
            event.accepted = true
        }
    }
    Accessible.role: Accessible.CheckBox
    Accessible.checkable: true
    Accessible.checked: on
    Accessible.onToggleAction: toggle.toggled(!toggle.on)
    Accessible.onPressAction: toggle.toggled(!toggle.on)

    implicitWidth: 42
    implicitHeight: 24
    radius: Theme.radius
    color: on ? Theme.action : Theme.line
    opacity: enabled ? 1 : 0.4
    Behavior on color { ColorAnimation { duration: Theme.quick } }

    Rectangle {
        x: toggle.on ? toggle.width - width - 3 : 3
        anchors.verticalCenter: parent.verticalCenter
        width: 18
        height: 18
        radius: 2
        color: toggle.on ? Theme.actionInk : Theme.text
        Behavior on x { NumberAnimation { duration: Theme.quick; easing.type: Easing.OutCubic } }
    }
    MouseArea {
        anchors.fill: parent
        anchors.margins: -6
        cursorShape: Qt.PointingHandCursor
        onClicked: toggle.toggled(!toggle.on)
    }
    FocusRing {}
}

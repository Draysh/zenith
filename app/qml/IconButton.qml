import QtQuick

// A square button that is only an icon. `tip` says what it does.
Rectangle {
    id: button
    property string icon: ""
    property string tip: ""
    property bool active: false
    property bool filled: false
    property color tint: active ? Theme.accent : Theme.textDim
    property real iconSize: 18
    signal clicked()

    // Reachable with Tab; Space or Enter presses it.
    activeFocusOnTab: true
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            button.clicked()
            event.accepted = true
        }
    }
    Accessible.role: Accessible.Button
    Accessible.name: tip !== "" ? tip : icon
    Accessible.onPressAction: button.clicked()

    implicitWidth: 34
    implicitHeight: 34
    radius: Theme.radius
    color: area.pressed ? Theme.hover : area.containsMouse ? Theme.raised : "transparent"
    opacity: enabled ? 1 : 0.35
    Behavior on color { ColorAnimation { duration: Theme.quick } }

    Icon {
        anchors.centerIn: parent
        name: button.icon
        size: button.iconSize
        filled: button.filled
        color: area.containsMouse && !button.active ? Theme.text : button.tint
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: button.clicked()
    }

    FocusRing {}
    Tip { shown: (area.containsMouse || button.activeFocus) && button.tip !== ""; text: button.tip }
}

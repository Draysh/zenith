import QtQuick

// A button with words. `primary` is the one main action of a screen.
Rectangle {
    id: button
    property string text: ""
    property string icon: ""
    property bool primary: false
    // Said after the pointer rests on it; needed when there are no words.
    property string tip: ""
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
    Accessible.name: text !== "" ? text : tip
    Accessible.onPressAction: button.clicked()

    implicitWidth: row.implicitWidth + 28
    implicitHeight: 36
    radius: Theme.radius
    opacity: enabled ? 1 : 0.4
    color: primary ? (area.containsMouse ? Theme.actionHover : Theme.action)
                   : (area.pressed ? Theme.hover : area.containsMouse ? Theme.raised : "transparent")
    border.color: primary ? "transparent" : area.containsMouse ? Theme.action : Theme.textFaint
    Behavior on color { ColorAnimation { duration: Theme.quick } }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 8
        Icon {
            visible: button.icon !== ""
            name: button.icon
            size: 16
            color: button.primary ? Theme.actionInk : Theme.text
            anchors.verticalCenter: parent.verticalCenter
        }
        Text {
            visible: button.text !== ""
            text: button.text
            color: button.primary ? Theme.actionInk : Theme.text
            font.family: Theme.family
            font.pixelSize: Theme.body
            font.weight: Font.DemiBold
            anchors.verticalCenter: parent.verticalCenter
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: button.clicked()
    }

    FocusRing {}
    Tip { shown: area.containsMouse && button.tip !== ""; text: button.tip }
}

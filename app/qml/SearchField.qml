import QtQuick

Rectangle {
    id: field
    property alias text: input.text
    property string placeholder: "Search"

    implicitWidth: 260
    implicitHeight: 36
    radius: Theme.radius
    color: Theme.surface
    border.color: input.activeFocus ? Theme.action : Theme.line
    Behavior on border.color { ColorAnimation { duration: Theme.quick } }

    Icon {
        id: glass
        anchors.left: parent.left
        anchors.leftMargin: 11
        anchors.verticalCenter: parent.verticalCenter
        name: "search"
        size: 15
        color: Theme.textFaint
    }
    TextInput {
        id: input
        anchors.left: glass.right
        anchors.leftMargin: 8
        anchors.right: clear.left
        anchors.verticalCenter: parent.verticalCenter
        color: Theme.text
        selectionColor: Theme.action
        selectedTextColor: Theme.actionInk
        clip: true
        font.family: Theme.family
        font.pixelSize: Theme.body
        Keys.onEscapePressed: { text = ""; focus = false }
        Text {
            visible: input.text === "" && !input.activeFocus
            text: field.placeholder
            color: Theme.textFaint
            font: input.font
        }
    }
    IconButton {
        id: clear
        visible: input.text !== ""
        anchors.right: parent.right
        anchors.rightMargin: 3
        anchors.verticalCenter: parent.verticalCenter
        implicitWidth: 28
        implicitHeight: 28
        icon: "x"
        iconSize: 14
        onClicked: input.text = ""
    }
}

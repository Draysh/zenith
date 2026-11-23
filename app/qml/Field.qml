import QtQuick

// A line of text to type in. `value` is what is kept; `committed` when
// Enter is pressed or the field is left. While it is being typed in, a
// change to `value` (the settings being saved and read back) does not
// replace what is there.
Rectangle {
    id: field
    property string value: ""
    property alias text: input.text
    property string placeholder: ""
    signal committed(string text)
    // Enter was pressed in it (after `committed`).
    signal entered()

    implicitWidth: 300
    implicitHeight: 36
    radius: Theme.radius
    color: Theme.surface
    border.color: input.activeFocus ? Theme.action : Theme.line
    Behavior on border.color { ColorAnimation { duration: Theme.quick } }

    // Put the keyboard in it, with what is there picked out to type over.
    function edit() { input.forceActiveFocus(); input.selectAll() }

    onValueChanged: if (!input.activeFocus) { input.text = value }
    Component.onCompleted: input.text = value

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        verticalAlignment: TextInput.AlignVCenter
        color: Theme.text
        selectionColor: Theme.action
        selectedTextColor: Theme.actionInk
        selectByMouse: true
        clip: true
        font.family: Theme.family
        font.pixelSize: Theme.body
        Accessible.name: field.placeholder
        onAccepted: field.entered()
        onEditingFinished: {
            field.committed(text)
            // Left the field: show what was kept (a refused value goes back).
            if (!activeFocus) { text = field.value }
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: input.text === "" && !input.activeFocus
            text: field.placeholder
            color: Theme.textFaint
            font: input.font
        }
    }
}

import QtQuick

// A short list to pick one thing from, all of it in view: the lines one
// under the other, the chosen one marked.
Column {
    id: list
    // [{ name, label }]
    property var options: []
    property string value: ""
    signal chosen(string name)

    spacing: 2
    Repeater {
        model: list.options
        Rectangle {
            id: line
            required property var modelData
            readonly property bool on: list.value === modelData.name
            width: list.width
            height: 34
            radius: Theme.radius
            color: pick.containsMouse ? Theme.hover : "transparent"
            Behavior on color { ColorAnimation { duration: Theme.quick } }

            // Reachable with Tab; Space or Enter picks it.
            activeFocusOnTab: true
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    list.chosen(modelData.name)
                    event.accepted = true
                }
            }
            Accessible.role: Accessible.RadioButton
            Accessible.name: modelData.label
            Accessible.checkable: true
            Accessible.checked: on
            Accessible.onPressAction: list.chosen(modelData.name)

            Rectangle {
                id: mark
                x: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 14
                height: 14
                radius: 2
                color: "transparent"
                border.width: 2
                border.color: line.on ? Theme.accent : Theme.textFaint
                Rectangle { anchors.centerIn: parent; width: 6; height: 6; radius: 1; color: Theme.accent; visible: line.on }
            }
            Text {
                anchors.left: mark.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                text: line.modelData.label
                elide: Text.ElideRight
                color: line.on ? Theme.text : Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.body
                font.weight: line.on ? Font.DemiBold : Font.Normal
            }
            MouseArea {
                id: pick
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: list.chosen(line.modelData.name)
            }
            FocusRing { anchors.margins: -1; radius: line.radius + 1 }
        }
    }
}

import QtQuick

// A few choices side by side, one of them chosen.
Rectangle {
    id: control
    // [{ key, label, icon }]
    property var options: []
    property string value: ""
    signal chosen(string key)

    implicitWidth: row.implicitWidth + 6
    implicitHeight: 36
    radius: Theme.radius
    color: Theme.surface
    border.color: Theme.line

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 2
        Repeater {
            model: control.options
            Rectangle {
                id: choice
                required property var modelData
                readonly property bool on: control.value === modelData.key
                activeFocusOnTab: true
                Keys.onPressed: event => {
                    if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                        control.chosen(modelData.key)
                        event.accepted = true
                    }
                }
                Accessible.role: Accessible.RadioButton
                Accessible.name: modelData.label
                Accessible.checkable: true
                Accessible.checked: on
                Accessible.onPressAction: control.chosen(modelData.key)
                width: content.implicitWidth + 24
                height: 30
                radius: 2
                color: on ? Theme.accent : pick.containsMouse ? Theme.hover : "transparent"
                Behavior on color { ColorAnimation { duration: Theme.quick } }
                Row {
                    id: content
                    anchors.centerIn: parent
                    spacing: 7
                    Icon {
                        visible: !!modelData.icon
                        name: modelData.icon || ""
                        size: 15
                        color: on ? Theme.accentInk : Theme.textDim
                        anchors.verticalCenter: parent.verticalCenter
                    }
                    Text {
                        text: modelData.label
                        color: on ? Theme.accentInk : pick.containsMouse ? Theme.text : Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.body
                        font.weight: on ? Font.DemiBold : Font.Normal
                        anchors.verticalCenter: parent.verticalCenter
                    }
                }
                MouseArea {
                    id: pick
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: control.chosen(modelData.key)
                }
                FocusRing { anchors.margins: -1; radius: choice.radius + 1 }
            }
        }
    }
}

import QtQuick

// A titled card of settings.
Column {
    id: group
    property string title: ""
    default property alias rows: card.data
    width: parent ? parent.width : 0
    spacing: 10

    // The group's name, with a rule running on from it to the edge.
    Item {
        width: parent.width
        height: name.implicitHeight
        Text {
            id: name
            text: group.title
            color: Theme.text
            font.family: Theme.titleFamily
            font.pixelSize: 22
            font.capitalization: Font.AllUppercase
        }
        Rectangle {
            anchors.left: name.right
            anchors.leftMargin: 14
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            height: 1
            color: Theme.line
        }
    }
    Rectangle {
        width: parent.width
        height: card.implicitHeight
        radius: Theme.radiusLarge
        color: Theme.surface
        border.color: Theme.line
        clip: true
        Column {
            id: card
            width: parent.width
        }
    }
}

import QtQuick

// The name over a part of a side panel, as a group of settings is named:
// in the title's letters, with a rule running on from it to the edge.
Item {
    id: title
    property string text: ""
    width: parent ? parent.width : 0
    implicitHeight: 48
    Text {
        id: name
        x: 14
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 6
        text: title.text
        color: Theme.text
        font.family: Theme.titleFamily
        font.pixelSize: 20
        font.capitalization: Font.AllUppercase
    }
    Rectangle {
        anchors.left: name.right
        anchors.leftMargin: 12
        anchors.right: parent.right
        anchors.rightMargin: 14
        anchors.verticalCenter: name.verticalCenter
        height: 1
        color: Theme.line
    }
}

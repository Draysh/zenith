import QtQuick

// One setting: what it is on the left, the control for it on the right.
Item {
    id: row
    property string label: ""
    property string note: ""
    default property alias control: slot.data
    // Set for a control too wide to sit beside the words: it goes under them.
    property bool stacked: false

    width: parent ? parent.width : 0
    implicitHeight: Math.max(56, words.implicitHeight + 24) + (stacked ? slot.childrenRect.height + 10 : 0)

    Column {
        id: words
        anchors.left: parent.left
        anchors.leftMargin: 18
        anchors.right: row.stacked ? parent.right : slot.left
        anchors.rightMargin: 18
        y: row.stacked ? 12 : (row.height - implicitHeight) / 2
        spacing: 3
        Text {
            width: parent.width
            text: row.label
            color: Theme.text
            font.family: Theme.family
            font.pixelSize: Theme.body
            font.weight: Font.Medium
        }
        Text {
            visible: row.note !== ""
            width: parent.width
            text: row.note
            wrapMode: Text.WordWrap
            color: Theme.textFaint
            font.family: Theme.family
            font.pixelSize: Theme.small
            lineHeight: 1.25
        }
    }
    Item {
        id: slot
        width: row.stacked ? parent.width - 36 : childrenRect.width
        height: childrenRect.height
        x: row.stacked ? 18 : parent.width - width - 18
        y: row.stacked ? words.y + words.implicitHeight + 10 : (row.height - height) / 2
    }
    Rectangle { anchors.bottom: parent.bottom; x: 18; width: parent.width - 36; height: 1; color: Theme.line; opacity: 0.5 }
}

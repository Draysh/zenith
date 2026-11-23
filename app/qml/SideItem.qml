import QtQuick
import Zenith

// A row of the library's sidebar: a place to look (all clips, a game).
Rectangle {
    id: item
    property string label: ""
    property string icon: ""
    property string cover: ""
    property string count: ""
    property bool selected: false
    signal clicked()

    // Reachable with Tab; Space or Enter presses it.
    activeFocusOnTab: true
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            item.clicked()
            event.accepted = true
        }
    }
    Accessible.role: Accessible.Button
    Accessible.name: label
    Accessible.onPressAction: item.clicked()

    width: parent ? parent.width : 0
    // A long name goes onto a second line, and the row grows for it.
    height: Math.max(40, name.implicitHeight + 14)
    radius: Theme.radius
    color: selected ? Theme.raised : area.containsMouse ? Theme.surface : "transparent"
    Behavior on color { ColorAnimation { duration: Theme.quick } }

    Rectangle {
        visible: item.selected
        width: 3
        height: 18
        radius: 2
        color: Theme.accent
        anchors.verticalCenter: parent.verticalCenter
    }

    Item {
        id: lead
        width: 24
        height: 24
        anchors.left: parent.left
        anchors.leftMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        Icon {
            anchors.centerIn: parent
            visible: item.cover === ""
            name: item.icon
            size: 17
            color: item.selected ? Theme.accentWords : Theme.textDim
        }
        Rectangle {
            visible: item.cover !== ""
            anchors.centerIn: parent
            width: 20
            height: 26
            radius: 4
            color: Theme.surface
            clip: true
            Image {
                anchors.fill: parent
                source: Library.fileUrl(item.cover)
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                sourceSize.height: 60
            }
        }
    }
    Text {
        id: name
        anchors.left: lead.right
        anchors.leftMargin: 10
        anchors.right: number.left
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: item.label
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        maximumLineCount: 2
        lineHeight: 1.1
        elide: Text.ElideRight
        color: item.selected ? Theme.text : Theme.textDim
        font.family: Theme.family
        font.pixelSize: Theme.body
        font.weight: item.selected ? Font.DemiBold : Font.Normal
    }
    Text {
        id: number
        anchors.right: parent.right
        anchors.rightMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: item.count
        color: Theme.textFaint
        font.family: Theme.monoFamily
        font.pixelSize: Theme.small
        font.features: { "tnum": 1 }
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: item.clicked()
    }
    FocusRing { anchors.margins: -1; radius: item.radius + 1 }
    // One too long even for two lines is said in full.
    Tip { shown: area.containsMouse && name.truncated; text: item.label }
}

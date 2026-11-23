import QtQuick
import Zenith

// What can be done to the clips that are picked, all at once: one press
// each. It sits over the foot of the library while any are picked.
Rectangle {
    id: bar
    property int count: 0
    // Whether every picked clip is a favourite: the star then takes it away.
    property bool allFavorite: false
    // Room for the words beside the icons.
    property bool wide: true
    signal favorite()
    signal tag()
    signal move()
    signal copy()
    signal trash()
    signal cleared()

    width: row.implicitWidth + 24
    height: 52
    radius: Theme.radiusLarge
    color: Theme.raised
    border.color: Theme.accent
    opacity: count > 0 ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.quick } }

    // Clicks on the bar are not clicks on the cards under it.
    MouseArea { anchors.fill: parent; hoverEnabled: true }

    component Act: Btn {
        property string label: ""
        text: bar.wide ? label : ""
        tip: bar.wide ? "" : label
        implicitHeight: 34
        anchors.verticalCenter: parent.verticalCenter
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 8
        Text {
            anchors.verticalCenter: parent.verticalCenter
            rightPadding: 6
            text: bar.count + " selected"
            color: Theme.accent
            font.family: Theme.monoFamily
            font.pixelSize: Theme.small
            font.letterSpacing: 1.4
            font.capitalization: Font.AllUppercase
        }
        Act { icon: "star"; label: bar.allFavorite ? "Unfavourite" : "Favourite"; onClicked: bar.favorite() }
        Act { icon: "tag"; label: "Tag"; onClicked: bar.tag() }
        Act { icon: "move"; label: "Move"; onClicked: bar.move() }
        Act { icon: "copy"; label: "Copy"; onClicked: bar.copy() }
        Act { icon: "trash"; label: "Trash"; onClicked: bar.trash() }
        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            icon: "x"
            tip: "Clear the selection (Esc)"
            onClicked: bar.cleared()
        }
    }
}

import QtQuick
import Zenith

// The order the clips are in: a button that opens the short list of
// orders under itself.
IconButton {
    id: button
    // [{ name, label }]
    property var options: []
    property string value: ""
    signal chosen(string name)

    readonly property var now: options.find(o => o.name === value) || { label: "" }
    property bool open: false

    icon: "sort"
    // Lit when the clips are not in the order they usually are.
    active: options.length > 0 && value !== options[0].name
    tip: open ? "" : "Sorted: " + now.label.toLowerCase()
    // The list is drawn over everything else in the window, at the
    // button's right edge.
    property point corner: Qt.point(0, 0)
    onClicked: { corner = mapToItem(null, width, height); open = !open }
    Keys.onEscapePressed: event => { if (open) { open = false } else { event.accepted = false } }

    // A click anywhere else closes the list.
    MouseArea {
        parent: button.Window.contentItem
        anchors.fill: parent
        visible: button.open
        z: 900
        onClicked: button.open = false
        onWheel: wheel => { button.open = false; wheel.accepted = false }
    }
    Rectangle {
        parent: button.Window.contentItem
        visible: button.open
        z: 901
        x: button.corner.x - width
        y: button.corner.y + 6
        width: 190
        height: list.implicitHeight + 12
        radius: Theme.radius
        color: Theme.raised
        border.color: Theme.line
        Choices {
            id: list
            x: 6
            y: 6
            width: parent.width - 12
            options: button.options
            value: button.value
            onChosen: name => { button.open = false; button.chosen(name) }
        }
    }
}

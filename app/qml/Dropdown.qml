import QtQuick

// One thing chosen from a few, on one line: pressed, the others open under
// it. The list is drawn over the whole window and not inside its parent,
// as a Tip is, so no box the line sits in cuts it off.
Rectangle {
    id: drop
    // [{ key, label, note, side, bad }]: `note` goes under the label in
    // the list, `side` at the right (a size, say); `bad` writes the note
    // as something wrong.
    property var options: []
    property string value: ""
    property bool open: false
    // How wide the list that opens is; it opens from the line's left edge.
    property real listWidth: width
    signal chosen(string key)
    readonly property int at: Math.max(0, options.findIndex(o => o.key === value))
    readonly property var current: options[at] || { key: "", label: "", side: "" }

    function show() {
        const root = drop.Window.contentItem
        if (!root) { return }
        sheet.parent = root
        const under = drop.mapToItem(root, 0, drop.height + 4)
        list.x = Math.round(under.x)
        // Over the line where there is no room under it.
        list.y = Math.round(under.y + list.height <= root.height - 6 ? under.y : under.y - drop.height - 8 - list.height)
        open = true
    }
    function pick(key) {
        open = false
        if (key !== value) { drop.chosen(key) }
    }

    implicitWidth: 240
    implicitHeight: 40
    radius: Theme.radius
    color: area.containsMouse || open ? Theme.raised : Theme.surface
    border.color: open ? Theme.action : Theme.line
    opacity: enabled ? 1 : 0.5
    Behavior on color { ColorAnimation { duration: Theme.quick } }
    onEnabledChanged: if (!enabled) { open = false }

    // Reachable with Tab; Space or Enter opens it, the arrows go through
    // the choices, Escape closes it.
    activeFocusOnTab: true
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { open ? open = false : show() }
        else if (event.key === Qt.Key_Down && at < options.length - 1) { drop.chosen(options[at + 1].key) }
        else if (event.key === Qt.Key_Up && at > 0) { drop.chosen(options[at - 1].key) }
        else if (event.key === Qt.Key_Escape && open) { open = false }
        else { return }
        event.accepted = true
    }
    Accessible.role: Accessible.ComboBox
    Accessible.name: current.label
    Accessible.onPressAction: open ? open = false : show()

    Text {
        anchors.left: parent.left
        anchors.leftMargin: 10
        anchors.right: side.left
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: drop.current.label
        elide: Text.ElideRight
        color: Theme.text
        font.family: Theme.family
        font.pixelSize: drop.height < 36 ? 13 : Theme.body
        font.weight: Font.DemiBold
    }
    Text {
        id: side
        anchors.right: arrow.left
        anchors.rightMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        text: drop.current.side || ""
        color: Theme.textDim
        font.family: Theme.monoFamily
        font.pixelSize: Theme.small
    }
    Icon {
        id: arrow
        anchors.right: parent.right
        anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        name: "chevron"
        size: 16
        rotation: drop.open ? 180 : 0
        color: Theme.textDim
        Behavior on rotation { NumberAnimation { duration: Theme.quick } }
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: drop.open ? drop.open = false : drop.show()
    }
    FocusRing {}

    Item {
        id: sheet
        anchors.fill: parent
        z: 900
        visible: drop.open
        // The window was sized under it: the list is no longer under its line.
        onWidthChanged: drop.open = false
        onHeightChanged: drop.open = false
        // A press anywhere else closes it, and goes no further.
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onPressed: drop.open = false; onWheel: wheel => wheel.accepted = true }
        Rectangle {
            id: list
            width: drop.listWidth
            height: lines.implicitHeight + 2
            radius: Theme.radius
            color: Theme.surface
            border.color: Theme.line
            clip: true
            Column {
                id: lines
                x: 1
                y: 1
                width: parent.width - 2
                Repeater {
                    model: drop.options
                    Rectangle {
                        id: line
                        required property var modelData
                        required property int index
                        readonly property bool on: drop.value === modelData.key
                        width: lines.width
                        height: 48
                        color: pick.containsMouse ? Theme.hover : on ? Theme.raised : "transparent"
                        Rectangle { visible: line.index > 0; width: parent.width; height: 1; color: Theme.line; opacity: 0.6 }
                        Rectangle { visible: line.on; width: 3; height: parent.height; color: Theme.accent }
                        Column {
                            anchors.left: parent.left
                            anchors.leftMargin: 14
                            anchors.right: lineSide.left
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 2
                            Text {
                                text: line.modelData.label
                                color: Theme.text
                                font.family: Theme.family
                                font.pixelSize: Theme.body
                                font.weight: Font.DemiBold
                            }
                            Text {
                                visible: text !== ""
                                width: parent.width
                                text: line.modelData.note || ""
                                elide: Text.ElideRight
                                color: line.modelData.bad ? Theme.record : Theme.textFaint
                                font.family: Theme.family
                                font.pixelSize: Theme.small
                            }
                        }
                        Text {
                            id: lineSide
                            anchors.right: parent.right
                            anchors.rightMargin: 14
                            anchors.verticalCenter: parent.verticalCenter
                            text: line.modelData.side || ""
                            color: line.on ? Theme.text : Theme.textFaint
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.small
                        }
                        MouseArea {
                            id: pick
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: drop.pick(line.modelData.key)
                        }
                    }
                }
            }
        }
    }
}

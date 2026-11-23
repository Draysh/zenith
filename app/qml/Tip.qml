import QtQuick

// A small label by its parent, shown after the pointer rests on it. It is
// drawn over the whole window and not inside its parent, so no box the
// parent sits in cuts it off; it goes under its parent, or over it where
// there is no room under, and stays inside the window.
Item {
    id: tip
    property bool shown: false
    property alias text: label.text
    property bool visibleNow: false

    anchors.fill: parent

    onShownChanged: shown ? delay.restart() : (delay.stop(), visibleNow = false)
    Timer { id: delay; interval: 500; onTriggered: { tip.place(); tip.visibleNow = true } }
    // What it is on may move under it (a page scrolled by the wheel with
    // the pointer at rest): the label goes along.
    Timer { interval: 60; repeat: true; running: tip.visibleNow; onTriggered: tip.place() }

    function place() {
        const root = tip.Window.contentItem
        if (!root) { return }
        bubble.parent = root
        const under = tip.mapToItem(root, tip.width / 2, tip.height + 6)
        const over = tip.mapToItem(root, tip.width / 2, -6)
        bubble.x = Math.round(Math.max(6, Math.min(root.width - bubble.width - 6, under.x - bubble.width / 2)))
        bubble.y = Math.round(under.y + bubble.height <= root.height - 6 ? under.y : over.y - bubble.height)
    }

    Rectangle {
        id: bubble
        z: 1000
        visible: opacity > 0
        opacity: tip.visibleNow && tip.visible ? 1 : 0
        width: label.implicitWidth + 16
        height: label.implicitHeight + 10
        radius: Theme.radius
        color: "#F2000000"
        border.color: Theme.line
        Behavior on opacity { NumberAnimation { duration: Theme.quick } }

        Text {
            id: label
            textFormat: Text.PlainText
            anchors.centerIn: parent
            color: "#EEF4F6"
            font.family: Theme.family
            font.pixelSize: Theme.small
        }
    }
}

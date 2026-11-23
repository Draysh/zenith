import QtQuick

// The outline around whichever control the keyboard is on. Clicking does
// not move the keyboard's place, so it only shows when Tab is used.
Rectangle {
    anchors.fill: parent
    anchors.margins: -3
    radius: (parent.radius || 0) + 3
    color: "transparent"
    border.width: 2
    border.color: Theme.action
    visible: parent.activeFocus
}

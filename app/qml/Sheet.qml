import QtQuick

// A panel over the page for one question or a few fields. Escape, or a
// click beside it, closes it without doing anything.
FocusScope {
    id: sheet
    property string title: ""
    property bool open: false
    property int panelWidth: 460
    default property alias content: body.data
    signal closed()

    function show() { open = true; forceActiveFocus() }
    function close() { if (open) { open = false; closed() } }

    anchors.fill: parent
    visible: open
    z: 500
    Keys.onEscapePressed: close()
    Accessible.role: Accessible.Dialog
    Accessible.name: title

    Rectangle {
        anchors.fill: parent
        color: "#C006090B"
        // Nothing under the sheet is pressed, hovered or scrolled.
        MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            onClicked: sheet.close()
            onWheel: wheel => wheel.accepted = true
        }
    }
    Rectangle {
        anchors.centerIn: parent
        width: Math.min(sheet.panelWidth, sheet.width - 48)
        height: column.implicitHeight + 48
        radius: Theme.radiusLarge
        color: Theme.surface
        border.color: Theme.line
        MouseArea { anchors.fill: parent }
        Column {
            id: column
            x: 24
            y: 22
            width: parent.width - 48
            spacing: 16
            Text {
                width: parent.width
                text: sheet.title
                color: Theme.accentWords
                wrapMode: Text.Wrap
                font.family: Theme.titleFamily
                font.pixelSize: Theme.display
                font.capitalization: Font.AllUppercase
            }
            Column {
                id: body
                width: parent.width
                spacing: 14
            }
        }
    }
}

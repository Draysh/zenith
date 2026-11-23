import QtQuick
import QtQuick.Dialogs
import Zenith

// Picks one of the app's colours: a few ready ones to click, any other
// from the system's colour window, or typed as #RRGGBB. `value` is "" for
// Zenith's own colour, which is the first of the ready ones.
Row {
    id: choice
    property string value: ""
    // Zenith's own colour for this place.
    property string standard: "#3DE0F5"
    // What the colour is for, for a screen reader.
    property string what: "Colour"
    signal chosen(string colour)

    // The ready ones after Zenith's own, as "#RRGGBB" in capitals, and
    // what each is called.
    property var others: ["#3DE0F5", "#FF3DA5", "#6FE08A", "#FFB454", "#FF5A3C", "#B58CFF", "#5B8CFF", "#F1EFE4"]
    property var names: ({
        "#3DE0F5": "Cyan", "#FF3DA5": "Magenta", "#6FE08A": "Green", "#FFB454": "Amber",
        "#FF5A3C": "Red", "#B58CFF": "Violet", "#5B8CFF": "Blue", "#F1EFE4": "Paper"
    })

    readonly property string shown: value !== "" ? value : standard
    readonly property var ready: [standard].concat(others.filter(c => c !== standard))
    function pick(colour) {
        const hex = String(colour).toUpperCase()
        choice.chosen(hex === standard.toUpperCase() ? "" : hex)
    }

    spacing: 6

    Repeater {
        model: choice.ready
        Rectangle {
            id: swatch
            required property string modelData
            required property int index
            readonly property bool on: choice.shown.toUpperCase() === modelData
            anchors.verticalCenter: parent.verticalCenter
            width: 26
            height: 26
            radius: 13
            color: "transparent"
            border.width: 2
            border.color: on ? Theme.text : "transparent"
            activeFocusOnTab: true
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    choice.pick(modelData)
                    event.accepted = true
                }
            }
            Accessible.role: Accessible.RadioButton
            Accessible.name: choice.what + ": " + (index === 0 ? "Zenith's own" : choice.names[modelData])
            Accessible.checkable: true
            Accessible.checked: on
            Accessible.onPressAction: choice.pick(modelData)
            Rectangle {
                anchors.centerIn: parent
                width: 18
                height: 18
                radius: 9
                color: swatch.modelData
                // A dark one still shows on a dark window.
                border.width: 1
                border.color: Theme.line
            }
            MouseArea {
                id: area
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: choice.pick(swatch.modelData)
            }
            FocusRing {}
            Tip { shown: area.containsMouse; text: swatch.index === 0 ? "Zenith's own" : choice.names[swatch.modelData] }
        }
    }
    Field {
        id: hex
        anchors.verticalCenter: parent.verticalCenter
        implicitWidth: 96
        value: choice.shown
        placeholder: "#RRGGBB"
        Accessible.name: choice.what + ", as #RRGGBB"
        onCommitted: text => {
            const typed = text.trim().replace(/^#?/, "#")
            if (/^#[0-9a-fA-F]{6}$/.test(typed) && typed.toUpperCase() !== choice.shown.toUpperCase()) { choice.pick(typed) }
        }
    }
    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "edit"
        tip: "Choose any colour"
        onClicked: { dialog.selectedColor = choice.shown; dialog.open() }
    }
    ColorDialog {
        id: dialog
        title: choice.what
        onAccepted: choice.pick(String(selectedColor).substring(0, 7))
    }
}

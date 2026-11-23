import QtQuick
import Zenith

// Picks a key combination by pressing it. Click it (or press Enter on it),
// then hold the modifiers and press the key: `Alt+F9`, `Ctrl+Shift+S`.
// Escape leaves it as it was.
Rectangle {
    id: picker
    // The combination as the settings spell it.
    property string value: ""
    // Something wrong with the last key pressed, said under the picker.
    property string problem: ""
    readonly property bool listening: activeFocus && armed
    property bool armed: false
    signal picked(string combination)

    // Reachable with Tab; Space or Enter starts listening.
    activeFocusOnTab: true
    Accessible.role: Accessible.Button
    Accessible.name: listening ? "Press the key to use" : "Clip key: " + value + ". Press to change."
    Accessible.onPressAction: { armed = true; forceActiveFocus() }
    onActiveFocusChanged: if (!activeFocus) { armed = false }

    implicitWidth: Math.max(180, label.implicitWidth + 32)
    implicitHeight: 36
    radius: Theme.radius
    color: listening ? Theme.raised : area.containsMouse ? Theme.raised : Theme.surface
    border.width: listening ? 2 : 1
    border.color: listening ? Theme.action : Theme.line
    Behavior on color { ColorAnimation { duration: Theme.quick } }

    // The name of the key of a press, the way the settings spell keys; ""
    // for one that cannot be a clip key (or is only a modifier).
    function keyName(event) {
        const k = event.key
        // The number pad's keys are their own: Windows tells its digits and
        // signs apart from the ones on the main block, so a combination made
        // with one must name it.
        if (event.modifiers & Qt.KeypadModifier) {
            if (k >= Qt.Key_0 && k <= Qt.Key_9) { return "Num" + String.fromCharCode(k) }
            const pad = {}
            pad[Qt.Key_Plus] = "NumPlus"
            pad[Qt.Key_Minus] = "NumMinus"
            pad[Qt.Key_Asterisk] = "NumMultiply"
            pad[Qt.Key_Slash] = "NumDivide"
            pad[Qt.Key_Period] = "NumDecimal"
            pad[Qt.Key_Comma] = "NumDecimal"
            if (pad[k]) { return pad[k] }
        }
        if (k >= Qt.Key_A && k <= Qt.Key_Z) { return String.fromCharCode(k) }
        if (k >= Qt.Key_0 && k <= Qt.Key_9) { return String.fromCharCode(k) }
        if (k >= Qt.Key_F1 && k <= Qt.Key_F24) { return "F" + (k - Qt.Key_F1 + 1) }
        const named = {}
        named[Qt.Key_Space] = "Space"
        named[Qt.Key_Insert] = "Insert"
        named[Qt.Key_Delete] = "Delete"
        named[Qt.Key_Home] = "Home"
        named[Qt.Key_End] = "End"
        named[Qt.Key_PageUp] = "PageUp"
        named[Qt.Key_PageDown] = "PageDown"
        named[Qt.Key_Print] = "PrintScreen"
        named[Qt.Key_ScrollLock] = "ScrollLock"
        named[Qt.Key_Pause] = "Pause"
        return named[k] || ""
    }
    function modifiers(event) {
        let held = ""
        if (event.modifiers & Qt.ControlModifier) { held += "Ctrl+" }
        if (event.modifiers & Qt.AltModifier) { held += "Alt+" }
        if (event.modifiers & Qt.ShiftModifier) { held += "Shift+" }
        if (event.modifiers & Qt.MetaModifier) { held += "Win+" }
        return held
    }

    Keys.onPressed: event => {
        if (!armed) {
            if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                armed = true
                problem = ""
                event.accepted = true
            }
            return
        }
        event.accepted = true
        if (event.key === Qt.Key_Escape) { armed = false; problem = ""; return }
        const lone = [Qt.Key_Control, Qt.Key_Alt, Qt.Key_Shift, Qt.Key_Meta, Qt.Key_AltGr, Qt.Key_Super_L, Qt.Key_Super_R]
        if (lone.indexOf(event.key) >= 0) { return }
        const key = keyName(event)
        if (key === "") { problem = "That key cannot be a clip key. Try a letter, a number, an F key or a number pad key."; return }
        const combination = modifiers(event) + key
        const wrong = Settings.hotkeyProblem(combination)
        if (wrong !== "") { problem = wrong; return }
        problem = ""
        armed = false
        picker.picked(combination)
    }

    Text {
        id: label
        anchors.centerIn: parent
        text: picker.listening ? "Press the key…" : picker.value
        color: picker.listening ? Theme.action : Theme.text
        font.family: picker.listening ? Theme.family : Theme.monoFamily
        font.pixelSize: Theme.body
        font.weight: Font.Medium
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: { picker.problem = ""; picker.armed = !picker.listening; picker.forceActiveFocus() }
    }
    FocusRing { visible: parent.activeFocus && !picker.listening }
}

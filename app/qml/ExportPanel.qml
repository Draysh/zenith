import QtQuick
import Zenith

// Exporting the edit, in the corner of the editor: pick where it has to
// fit from a list that opens, see what that will look like, and get a
// file that is ready to paste. As tall as what it holds.
Item {
    id: panel
    // Bumped by the editor whenever the edit changes, so the plans are worked out again.
    property int revision: 0
    // The clip this panel exports: progress and results of another clip's
    // export are not this panel's.
    property var clipId: -1
    // The clip's name, which the exported file gets too.
    property string title: ""
    signal renamed(string name)
    // Enter was pressed in the name: the keyboard is the editor's again.
    signal entered()
    property string choice: "discord"
    property int customMegabytes: 100
    property bool running: false
    property real progress: 0
    property var result: null
    property string problem: ""
    signal say(string message)

    readonly property var presets: [
        { key: "discord", label: "Discord", note: "Free accounts", megabytes: 20 },
        { key: "basic", label: "Nitro Basic", note: "", megabytes: 50 },
        { key: "nitro", label: "Nitro", note: "", megabytes: 500 },
        { key: "custom", label: "Custom size", note: "", megabytes: -1 },
        { key: "full", label: "Full quality", note: "A new clip in your library", megabytes: 0 }
    ]
    // The sizes as the list shows them: what each is for, and how the
    // clip would come out at it.
    readonly property var choices: presets.map(p => {
        const plan = describe(p)
        return { key: p.key, label: p.label, note: plan.text !== "" ? plan.text : p.note, bad: plan.bad, side: p.megabytes > 0 ? p.megabytes + " MB" : "" }
    })
    readonly property var plan: describe(chosen)
    function megabytes(preset) { return preset.megabytes < 0 ? customMegabytes : preset.megabytes }
    readonly property var chosen: presets.find(p => p.key === choice)

    function describe(preset) {
        revision
        const plan = JSON.parse(Editor.plan(megabytes(preset)))
        if (plan.error) { return { bad: true, text: plan.error } }
        if (plan.width === undefined) { return { bad: false, text: "" } }
        const shape = plan.height + "p · " + Math.round(plan.fps) + " fps"
        return { bad: false, text: plan.full ? shape + " · every audio track" : shape + " · ~" + Theme.size(plan.bytes) }
    }

    function start() {
        result = null
        problem = ""
        progress = 0
        if (Editor.exportBusy()) {
            problem = "The last export is still stopping. Try again in a moment."
            return
        }
        // A name still being typed is the clip's name first.
        panel.renamed(name.text)
        running = true
        Editor.exportStart(megabytes(chosen))
    }

    Connections {
        target: Editor
        function onExportProgress(clip, fraction) { if (clip === panel.clipId) { panel.progress = fraction } }
        function onExportFinished(clip, path, bytes, error) {
            if (clip !== panel.clipId && clip >= 0) { return }
            panel.running = false
            if (error === "cancelled") { return }
            if (error !== "") { panel.problem = error; return }
            panel.result = { path: path, bytes: bytes, full: panel.choice === "full" }
            if (panel.choice !== "full") {
                Library.copyToClipboard(path)
                panel.say("Exported and copied. Paste it into Discord.")
            } else {
                panel.say("Saved as a new clip.")
            }
        }
    }

    // `--sizes` opens the list of sizes at once, for checking its look.
    Timer { interval: 600; running: Qt.application.arguments.indexOf("--sizes") >= 0; onTriggered: sizes.show() }

    // The editor is closing: an export nobody is waiting for is stopped.
    Component.onDestruction: if (running) { Editor.exportCancel() }

    implicitHeight: head.height + column.implicitHeight + 22

    PanelTitle { id: head; text: "Export" }

    Column {
        id: column
        x: 14
        y: head.height + 10
        width: parent.width - 28
        spacing: 8

        // The clip's name, here as well as over the picture: the exported
        // file is called this and nothing more.
        Field {
            id: name
            width: parent.width
            height: 30
            value: panel.title
            placeholder: "Name of the clip"
            enabled: !panel.running
            opacity: enabled ? 1 : 0.5
            onCommitted: text => panel.renamed(text)
            // Enter gives the keyboard back to the editor's shortcuts.
            onEntered: panel.entered()
        }

        // Where it has to fit (and the size of one's own beside it), and
        // the button that does it, on one line.
        Row {
            visible: !panel.running
            width: parent.width
            spacing: 6
            Dropdown {
                id: sizes
                width: parent.width - go.width - 6 - (amountBox.visible ? amountBox.width + 6 : 0)
                height: 30
                // The list is as wide as the panel, whatever is beside its line.
                listWidth: column.width
                options: panel.choices
                value: panel.choice
                onChosen: key => { panel.choice = key; panel.result = null; panel.problem = "" }
            }
            Rectangle {
                id: amountBox
                visible: panel.choice === "custom"
                width: 64
                height: 30
                radius: Theme.radius
                color: Theme.surface
                border.width: 1
                border.color: amount.activeFocus ? Theme.action : Theme.line
                TextInput {
                    id: amount
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    anchors.right: unit.left
                    anchors.verticalCenter: parent.verticalCenter
                    text: panel.customMegabytes
                    color: Theme.text
                    selectionColor: Theme.action
                    selectedTextColor: Theme.actionInk
                    selectByMouse: true
                    validator: IntValidator { bottom: 1; top: 4000 }
                    font.family: Theme.family
                    font.pixelSize: Theme.small
                    Accessible.name: "Size in megabytes"
                    onTextEdited: {
                        const n = parseInt(text)
                        if (n > 0) { panel.customMegabytes = n }
                    }
                }
                Text {
                    id: unit
                    anchors.right: parent.right
                    anchors.rightMargin: 7
                    anchors.verticalCenter: parent.verticalCenter
                    text: "MB"
                    color: Theme.textFaint
                    font.family: Theme.family
                    font.pixelSize: 11
                }
            }
            Btn {
                id: go
                implicitHeight: 30
                primary: true
                icon: "export"
                text: panel.choice === "full" ? "Save" : "Export"
                tip: panel.choice === "full" ? "Save as a new clip in your library" : "Export and copy, ready to paste"
                enabled: !panel.plan.bad
                onClicked: panel.start()
            }
        }
        // How the clip comes out at that size.
        Text {
            width: parent.width
            visible: text !== "" && !panel.running
            text: panel.plan.text !== "" ? panel.plan.text : panel.chosen.note
            wrapMode: Text.WordWrap
            color: panel.plan.bad ? Theme.record : Theme.textFaint
            font.family: Theme.family
            font.pixelSize: Theme.small
        }

        // Under way: how far it is, and the way out, on that same line.
        Item {
            visible: panel.running
            width: parent.width
            height: 30
            Rectangle {
                anchors.left: parent.left
                anchors.right: percent.left
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                height: 4
                radius: 2
                color: Theme.line
                Rectangle {
                    width: parent.width * panel.progress
                    height: parent.height
                    radius: 2
                    color: Theme.action
                    Behavior on width { NumberAnimation { duration: 150 } }
                }
            }
            Text {
                id: percent
                anchors.right: cancel.left
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                width: 36
                horizontalAlignment: Text.AlignRight
                text: Math.round(panel.progress * 100) + "%"
                color: Theme.textDim
                font.family: Theme.monoFamily
                font.pixelSize: Theme.small
                font.features: { "tnum": 1 }
            }
            Btn { id: cancel; anchors.right: parent.right; implicitHeight: 30; text: "Cancel"; onClicked: Editor.exportCancel() }
        }

        Text {
            visible: panel.problem !== ""
            width: parent.width
            wrapMode: Text.WordWrap
            text: panel.problem
            color: Theme.record
            font.family: Theme.family
            font.pixelSize: Theme.small
        }

        Rectangle {
            visible: panel.result !== null
            width: parent.width
            height: done.implicitHeight + 28
            radius: Theme.radius
            color: Theme.surface
            border.color: Theme.line
            Column {
                id: done
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.margins: 14
                spacing: 10
                Row {
                    spacing: 8
                    Icon { name: "check"; color: Theme.good; size: 18 }
                    Text {
                        text: panel.result ? (panel.result.full ? "Saved to your library" : "Ready to paste") + "  ·  " + Theme.size(panel.result.bytes) : ""
                        color: Theme.text
                        font.family: Theme.family
                        font.pixelSize: Theme.body
                        font.weight: Font.DemiBold
                    }
                }
                // The file itself, to drag into a chat or a folder.
                Rectangle {
                    id: chip
                    width: parent.width
                    height: 40
                    radius: Theme.radius
                    color: grab.containsMouse ? Theme.hover : Theme.raised
                    border.color: Theme.line
                    Drag.active: grab.drag.active
                    Drag.dragType: Drag.Automatic
                    Drag.supportedActions: Qt.CopyAction
                    Drag.mimeData: ({ "text/uri-list": panel.result ? Library.fileUrl(panel.result.path) + "\r\n" : "" })
                    Icon { id: chipIcon; x: 10; anchors.verticalCenter: parent.verticalCenter; name: "film"; size: 16 }
                    Text {
                        anchors.left: chipIcon.right
                        anchors.leftMargin: 8
                        anchors.right: parent.right
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        // Either slash: Windows writes its paths with the other one.
                        text: panel.result ? panel.result.path.replace(/^.*[\\/]/, "") : ""
                        elide: Text.ElideMiddle
                        color: Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.small
                    }
                    MouseArea {
                        id: grab
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.OpenHandCursor
                        drag.target: dragGhost
                    }
                    Item { id: dragGhost }
                    Tip { shown: grab.containsMouse && !grab.pressed; text: "Drag this into Discord or a folder" }
                }
                Row {
                    spacing: 8
                    Btn {
                        implicitHeight: 32
                        icon: "copy"
                        text: "Copy again"
                        onClicked: { Library.copyToClipboard(panel.result.path); panel.say("Copied.") }
                    }
                    Btn {
                        implicitHeight: 32
                        icon: "folder"
                        text: "Show"
                        onClicked: Qt.openUrlExternally(Library.fileUrl(panel.result.path.replace(/[\\/][^\\/]*$/, "")))
                    }
                }
            }
        }
    }
}

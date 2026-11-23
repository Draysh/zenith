import QtQuick
import Zenith

// What a clip is called and how it is found again: its name (and, when
// asked, its file's), its tags and its notes. Opened for several clips at
// once it only adds tags, to all of them.
Sheet {
    id: sheet
    // The clips it is for: one, or the ones picked.
    property var clips: []
    readonly property var one: clips.length === 1 ? clips[0] : null
    // The tags most used, to pick from instead of typing.
    property var known: []
    property bool withFile: false
    // Something was kept; `problem` is what went wrong, or empty.
    signal saved(string problem)

    title: one ? "Name, tags and notes" : "Tag " + clips.length + " clips"

    function edit(list) {
        clips = list
        known = JSON.parse(Library.tags()).slice(0, 14)
        withFile = false
        name.put(one ? one.title : "")
        tags.put(one ? one.tags.join(", ") : "")
        notes.text = one ? one.notes : ""
        show()
        if (one) { name.edit() } else { tags.edit() }
    }

    // The tags in the field, tidied as the library will keep them.
    function typed() {
        return tags.text.split(/[,#\n]/).map(t => t.trim().replace(/\s+/g, " ").toLowerCase()).filter(t => t !== "")
    }
    function toggle(tag) {
        const now = typed()
        const at = now.indexOf(tag)
        if (at >= 0) { now.splice(at, 1) } else { now.push(tag) }
        tags.put(now.join(", "))
    }

    function save() {
        let problem = ""
        if (one) {
            const title = name.text.trim()
            if (withFile) {
                problem = Library.renameWithFile(one.id, title)
            } else if (title !== one.title) {
                Library.rename(one.id, title)
            }
            Library.setTags(one.id, tags.text)
            Library.setNotes(one.id, notes.text)
        } else {
            Library.addTags(JSON.stringify(clips.map(c => c.id)), tags.text)
        }
        open = false
        saved(problem)
    }

    component Label: Text {
        color: Theme.textFaint
        font.family: Theme.monoFamily
        font.pixelSize: 11
        font.letterSpacing: 1.6
        font.capitalization: Font.AllUppercase
    }
    component Line: Field {
        width: parent.width
        // What is typed is what is kept, also once the field is left.
        function put(words) { value = words; text = words }
        onCommitted: words => value = words
        onEntered: sheet.save()
    }

    Column {
        visible: sheet.one !== null
        width: parent.width
        spacing: 6
        Label { text: "Name" }
        Line { id: name; placeholder: "The file's name" }
        Item {
            width: parent.width
            height: 30
            Toggle {
                id: fileToggle
                anchors.verticalCenter: parent.verticalCenter
                on: sheet.withFile
                onToggled: on => sheet.withFile = on
                Accessible.name: "Rename the file too"
            }
            Text {
                anchors.left: fileToggle.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                text: sheet.withFile ? "The file in the clip folder gets this name" : "Rename the file too"
                color: sheet.withFile ? Theme.text : Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.body
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: sheet.withFile = !sheet.withFile }
            }
        }
    }

    Column {
        width: parent.width
        spacing: 6
        Label { text: sheet.one ? "Tags" : "Tags to add" }
        Line { id: tags; placeholder: "ace, clutch, funny" }
        Flow {
            width: parent.width
            spacing: 6
            topPadding: 2
            Repeater {
                model: sheet.known
                Rectangle {
                    id: chip
                    required property var modelData
                    readonly property bool on: sheet.typed().indexOf(modelData.tag) >= 0
                    width: chipText.implicitWidth + 16
                    height: 24
                    radius: 2
                    color: on ? Theme.accent : chipArea.containsMouse ? Theme.hover : Theme.raised
                    activeFocusOnTab: true
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_Space) { sheet.toggle(modelData.tag); event.accepted = true }
                    }
                    Accessible.role: Accessible.CheckBox
                    Accessible.name: modelData.tag
                    Accessible.checked: on
                    Accessible.onPressAction: sheet.toggle(modelData.tag)
                    Text {
                        id: chipText
                        anchors.centerIn: parent
                        text: "#" + chip.modelData.tag
                        color: chip.on ? Theme.accentInk : Theme.textDim
                        font.family: Theme.monoFamily
                        font.pixelSize: 11
                        font.letterSpacing: 0.6
                    }
                    MouseArea {
                        id: chipArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: sheet.toggle(chip.modelData.tag)
                    }
                    FocusRing {}
                }
            }
        }
    }

    Column {
        visible: sheet.one !== null
        width: parent.width
        spacing: 6
        Label { text: "Notes" }
        Rectangle {
            width: parent.width
            height: 92
            radius: Theme.radius
            color: Theme.surface
            border.color: notes.activeFocus ? Theme.action : Theme.line
            Behavior on border.color { ColorAnimation { duration: Theme.quick } }
            Flickable {
                id: notesView
                anchors.fill: parent
                anchors.margins: 10
                contentHeight: notes.implicitHeight
                clip: true
                TextEdit {
                    id: notes
                    width: notesView.width
                    height: Math.max(notesView.height, implicitHeight)
                    wrapMode: TextEdit.Wrap
                    color: Theme.text
                    selectionColor: Theme.action
                    selectedTextColor: Theme.actionInk
                    selectByMouse: true
                    activeFocusOnTab: true
                    // Tab moves on to the buttons; it is not written.
                    KeyNavigation.priority: KeyNavigation.BeforeItem
                    KeyNavigation.tab: keep
                    font.family: Theme.family
                    font.pixelSize: Theme.body
                    Accessible.name: "Notes"
                    onCursorRectangleChanged: {
                        if (cursorRectangle.y < notesView.contentY) { notesView.contentY = cursorRectangle.y }
                        else if (cursorRectangle.y + cursorRectangle.height > notesView.contentY + notesView.height) {
                            notesView.contentY = cursorRectangle.y + cursorRectangle.height - notesView.height
                        }
                    }
                    Text {
                        visible: notes.text === "" && !notes.activeFocus
                        text: "What happened, who was there"
                        color: Theme.textFaint
                        font: notes.font
                    }
                }
            }
        }
    }

    Row {
        anchors.right: parent.right
        spacing: 10
        Btn { text: "Cancel"; onClicked: sheet.close() }
        Btn { id: keep; primary: true; text: sheet.one ? "Save" : "Add tags"; onClicked: sheet.save() }
    }
}

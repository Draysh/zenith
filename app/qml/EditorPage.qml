import QtQuick
import Zenith

// Editing one clip: the video, with export in the corner beside it and
// under that the clip and the library's others, down the window's whole
// height; under the video the things to do to the clip, and under those
// the timeline.
FocusScope {
    id: page
    required property var entry
    signal closed()
    // Another clip is to be opened in this one's place.
    signal openClip(var clip)
    signal say(string message)

    property var edit: opened()
    property string title: entry.title
    property int revision: 0
    // The pointer is dragging the playhead or a handle: playback does not
    // jump cuts while it does.
    property bool scrubbing: false
    // Playing the result: cut parts are skipped and it stops where the
    // result ends. Otherwise playback stops at the end of the part it is
    // in (`stopAt`), and playing again goes on with the next part.
    property bool previewing: false
    // Where playback that is not of the result stops: the end of the kept
    // part it is in or comes to next. Past the last one, the clip's end.
    property real stopAt: -1
    // A trim handle or a volume slider is held: nothing else edits meanwhile.
    property bool holding: false
    // Where the handle being dragged has put its edge, to land on exactly.
    property real edge: -1
    readonly property real frame: 1 / Math.max(1, entry.fps)

    // The clip's edit. A clip that has left the library (deleted from
    // another window) gets a plain stand-in, so the page can say so and
    // close instead of failing halfway.
    function opened() {
        const state = JSON.parse(Editor.open(entry.id))
        if (state.segments) { return state }
        return {
            missing: true, duration: entry.seconds, kept: entry.seconds, changed: false, canUndo: false, canRedo: false,
            segments: [{ start: 0, end: entry.seconds }], gains: entry.tracks.map(() => ({ volume: 1, muted: false })), audioOffsetMs: 0
        }
    }
    function gains() { return edit.gains.map(g => g.muted ? 0 : g.volume) }
    function apply(state) {
        const parts = edit.segments.length
        edit = JSON.parse(state)
        revision += 1
        // The parts were renumbered: what was chosen may be another part now.
        if (edit.segments.length !== parts || timeline.selected >= edit.segments.length) { timeline.selected = -1 }
        player.setTrackGains(gains())
        player.setAudioDelay((edit.audioOffsetMs || 0) / 1000)
        // The part being played may end somewhere else now.
        stopAt = stopFor(player.position)
    }
    function stopFor(t) {
        const part = edit.segments.find(s => t < s.end - frame * 1.5)
        return part ? part.end : edit.duration
    }
    // Go to a time, as playback or the playhead: it stops at the end of
    // the part that time is in.
    function go(t, exact) {
        stopAt = stopFor(t)
        player.seek(t, exact)
    }
    function rename(name) {
        name = name.trim()
        if (name === "" || name === title) { return }
        Library.rename(entry.id, name)
        title = name
    }
    function outputTime(t) {
        let out = 0
        for (const s of edit.segments) {
            if (t < s.start) { break }
            out += Math.min(t, s.end) - s.start
        }
        return out
    }
    function togglePlay() {
        if (!player.paused) { player.paused = true; return }
        // Past the end of the result, there is no more of it to play.
        if (previewing && Editor.playable(player.position + 0.001) < 0) { previewing = false }
        if (!previewing) {
            // Stopped at the end of a part: go on with the next one, or
            // from the first again after the last.
            const at = player.position
            const parts = edit.segments
            const done = parts.findIndex(s => at >= s.end - frame * 1.5 && at <= s.end + frame * 0.5)
            if (done >= 0) { go(parts[(done + 1) % parts.length].start, true) }
            else if (at >= edit.duration - frame) { go(parts[0].start, true) }
            else { stopAt = stopFor(at) }
        }
        player.paused = false
    }
    // Play the result from its start, as it will be exported.
    function playResult() {
        previewing = true
        player.seek(edit.segments[0].start, true)
        player.paused = false
    }
    function splitHere() {
        const parts = edit.segments.length
        apply(Editor.split(player.position))
        if (edit.segments.length === parts) {
            page.say("Nothing to split here: move the playhead into a kept part, away from its ends.")
        }
    }
    // An end of a part is held (a handle on the timeline), moved, and
    // let go: the picture follows it,
    // and the whole of it is one step to undo.
    function edgeGrab() {
        holding = true
        scrubbing = true
        previewing = false
        player.paused = true
        Editor.grab()
    }
    function edgeMove(index, start, t) {
        edit = JSON.parse(start ? Editor.setStart(index, t) : Editor.setEnd(index, t))
        const s = edit.segments[index]
        edge = start ? s.start : Math.max(s.start, s.end - frame)
        // Nearest keyframe while dragging: fast enough to follow.
        player.seek(edge, false)
    }
    function edgeRelease() {
        apply(Editor.release())
        scrubbing = false
        holding = false
        // Now the very frame the edge is on.
        player.seek(edge >= 0 ? edge : player.position, true)
        edge = -1
    }
    // While a volume slider moves, the slider, the waveform and what is
    // heard follow each step; the export plans are worked out on release.
    function volumeGrab() {
        // Another slider's hold is still open (the wheel's rests a moment): it ends here.
        if (holding) { edit = JSON.parse(Editor.release()) }
        holding = true
        Editor.grab()
    }
    function volumeMove(track, volume) {
        edit = JSON.parse(Editor.setVolume(track, volume))
        player.setTrackGains(gains())
    }
    function volumeRelease() {
        apply(Editor.release())
        holding = false
    }
    function toggleMute(track) { apply(Editor.setMuted(track, !edit.gains[track].muted)) }
    // The part the playhead is in starts (or ends) where the playhead is:
    // what is before (or after) it in that part is cut away.
    function trimHere(start) {
        const at = player.position
        const index = edit.segments.findIndex(s => at >= s.start && at < s.end)
        if (index < 0) {
            page.say("Move the playhead into a kept part first.")
            return
        }
        const kept = edit.kept
        // Ending here keeps the frame that is showing.
        apply(start ? Editor.setStart(index, at) : Editor.setEnd(index, at + frame))
        if (Math.abs(edit.kept - kept) < 0.0005) { page.say(start ? "The part already starts here." : "The part already ends here.") }
        else if (!start) { player.paused = true; go(Math.max(0, edit.segments[index].end - frame), true) }
    }
    // Go to the cut before the playhead (-1) or after it (1): where a
    // kept part starts, or its last frame.
    function toCut(way) {
        const at = player.position
        const cuts = []
        for (const s of edit.segments) { cuts.push(s.start, Math.max(s.start, s.end - frame)) }
        const next = way > 0 ? cuts.find(c => c > at + frame * 0.5) : cuts.reverse().find(c => c < at - frame * 0.5)
        if (next === undefined) { return }
        player.paused = true
        previewing = false
        go(next, true)
    }
    // Keep the frame that is showing as a picture beside the clip, and
    // put it on the clipboard to paste.
    function saveFrame() {
        player.paused = true
        const dir = entry.path.replace(/[\\/][^\\/]*$/, "")
        const slash = entry.path.indexOf("\\") >= 0 ? "\\" : "/"
        // The clip's name, without what a file's name cannot hold.
        const name = (title.replace(/[\\/:*?"<>|]/g, " ").trim() || "Clip") + " at " + Theme.timecode(player.position).replace(":", "-") + ".png"
        const file = dir + slash + name
        if (!player.saveFrame(file)) {
            page.say("The frame could not be saved.")
            return ""
        }
        Library.copyToClipboard(file)
        page.say("Frame saved beside the clip and copied.")
        return file
    }
    // Remove a part: the one given, else the one chosen, else the one
    // the playhead is in.
    function removeSelected(given) {
        let index = given !== undefined && given >= 0 ? given : timeline.selected
        if (index < 0) {
            index = edit.segments.findIndex(s => player.position >= s.start && player.position < s.end)
        }
        if (edit.segments.length <= 1) {
            page.say("Split the clip first (S), then remove a part.")
        } else if (index >= 0) {
            apply(Editor.remove(index))
            timeline.selected = -1
        } else {
            page.say("Move the playhead into the part to remove, or take hold of one of its handles.")
        }
    }

    // `--keep-frame` keeps a frame as a picture a moment after opening,
    // for checking that it can be.
    Timer {
        interval: 1500
        running: Qt.application.arguments.indexOf("--keep-frame") >= 0
        onTriggered: page.saveFrame()
    }

    // `--hop` goes on to the library's next clip a moment after opening,
    // for checking that one clip can be left for another.
    Timer {
        interval: 1500
        running: Qt.application.arguments.indexOf("--hop") >= 0
        onTriggered: {
            const clips = JSON.parse(Library.clips("{}"))
            const next = clips[clips.findIndex(c => c.id === page.entry.id) + 1]
            if (next) { page.openClip(next) }
        }
    }
    // `--split 1.5` splits the clip there a moment after opening, for a
    // look at a clip that is cut.
    Timer {
        readonly property int at: Qt.application.arguments.indexOf("--split")
        interval: 1200
        running: at >= 0
        onTriggered: page.apply(Editor.split(parseFloat(Qt.application.arguments[at + 1])))
    }

    focus: true
    // Closing with a handle still held: the drag ends as one undo step.
    Component.onDestruction: if (Editor.held()) { Editor.release() }
    Component.onCompleted: {
        forceActiveFocus()
        if (edit.missing) {
            // Once the window has this page in hand, so it hears the close.
            Qt.callLater(() => { page.say("This clip is no longer in the library."); page.closed() })
            return
        }
        player.setTrackGains(gains())
        player.setAudioDelay((edit.audioOffsetMs || 0) / 1000)
        player.volume = Editor.listening()
        // `--zoom 3` opens zoomed in, for checking the timeline's look.
        const at = Qt.application.arguments.indexOf("--zoom")
        if (at >= 0) { Qt.callLater(() => timeline.zoomTo(parseFloat(Qt.application.arguments[at + 1]), 300)) }
    }
    Keys.onPressed: event => {
        // Typing in a field: its keys are not shortcuts. Enter and Escape
        // leave the field, so the shortcuts work again.
        const focused = page.Window.activeFocusItem
        if (focused && focused !== page && focused.cursorPosition !== undefined) {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Escape) {
                page.forceActiveFocus()
                event.accepted = true
            }
            return
        }
        // A handle or slider is held: an edit now would pull the part out
        // from under the pointer, so the keys wait.
        if (page.holding) {
            event.accepted = true
            return
        }
        const ctrl = event.modifiers & Qt.ControlModifier
        const shift = event.modifiers & Qt.ShiftModifier
        if (event.key === Qt.Key_Space) { togglePlay() }
        else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { playResult() }
        else if (event.key === Qt.Key_S && !ctrl) { splitHere() }
        else if (event.key === Qt.Key_Q && !ctrl) { trimHere(true) }
        else if (event.key === Qt.Key_W && !ctrl) { trimHere(false) }
        else if (event.key === Qt.Key_Up) { toCut(-1) }
        else if (event.key === Qt.Key_Down) { toCut(1) }
        else if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) { removeSelected() }
        else if (event.key === Qt.Key_Z && ctrl && shift) { apply(Editor.redo()) }
        else if (event.key === Qt.Key_Z && ctrl) { apply(Editor.undo()) }
        else if (event.key === Qt.Key_Y && ctrl) { apply(Editor.redo()) }
        else if (event.key === Qt.Key_Left) { player.paused = true; shift ? player.seek(player.position - 1, true) : player.step(-1) }
        else if (event.key === Qt.Key_Right) { player.paused = true; shift ? player.seek(player.position + 1, true) : player.step(1) }
        else if (event.key === Qt.Key_Home) { go(edit.segments[0].start, true) }
        else if (event.key === Qt.Key_End) { player.paused = true; go(edit.segments[edit.segments.length - 1].end - page.frame, true) }
        else if (event.key === Qt.Key_Escape) { page.closed() }
        else { return }
        event.accepted = true
    }

    Item {
        id: header
        height: 56
        anchors.left: parent.left
        anchors.right: side.left
        IconButton {
            id: back
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            icon: "back"
            tip: "Back to the library (Esc)"
            onClicked: page.closed()
        }
        Column {
            anchors.left: back.right
            anchors.leftMargin: 10
            anchors.right: parent.right
            anchors.rightMargin: 20
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Item {
                width: parent.width
                height: 20
                Text {
                    id: titleText
                    visible: !titleEdit.visible
                    width: Math.min(implicitWidth, parent.width)
                    text: page.title
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    color: titleArea.containsMouse ? Theme.action : Theme.text
                    font.family: Theme.family
                    font.pixelSize: 15
                    font.weight: Font.DemiBold
                    MouseArea {
                        id: titleArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.IBeamCursor
                        onClicked: { titleEdit.text = page.title; titleEdit.visible = true; titleEdit.forceActiveFocus(); titleEdit.selectAll() }
                    }
                    Tip { shown: titleArea.containsMouse; text: "Rename" }
                }
                TextInput {
                    id: titleEdit
                    visible: false
                    width: parent.width
                    color: Theme.text
                    selectionColor: Theme.action
                    selectedTextColor: Theme.actionInk
                    selectByMouse: true
                    clip: true
                    font: titleText.font
                    function finish(keep) {
                        if (!visible) { return }
                        visible = false
                        if (keep) { page.rename(text) }
                        page.forceActiveFocus()
                    }
                    Keys.onReturnPressed: finish(true)
                    Keys.onEnterPressed: finish(true)
                    onActiveFocusChanged: if (!activeFocus) { finish(true) }
                    Keys.onEscapePressed: finish(false)
                }
            }
            Text {
                text: page.entry.game + "  ·  " + Theme.day(page.entry.savedAt) + " " + Theme.clock(page.entry.savedAt)
                textFormat: Text.PlainText
                color: Theme.textFaint
                font.family: Theme.family
                font.pixelSize: Theme.small
            }
        }
    }

    Rectangle {
        id: stage
        anchors.top: header.bottom
        anchors.bottom: transport.top
        anchors.left: parent.left
        anchors.right: side.left
        anchors.leftMargin: 20
        anchors.rightMargin: 20
        color: "#000000"
        clip: true

        MpvPlayer {
            id: player
            anchors.fill: parent
            source: page.entry.path
            onLoadedChanged: if (loaded) { seek(page.edit.segments.length ? page.edit.segments[0].start : 0, true) }
            onPositionChanged: {
                if (page.scrubbing || paused) { return }
                if (!page.previewing) {
                    // The end of the part: stop on its last frame.
                    if (page.stopAt >= 0 && page.stopAt < page.edit.duration - page.frame && position >= page.stopAt - page.frame * 0.5) {
                        paused = true
                        if (position >= page.stopAt) { seek(Math.max(0, page.stopAt - page.frame), true) }
                    }
                    return
                }
                const next = Editor.playable(position)
                if (next < 0) {
                    paused = true
                    page.previewing = false
                } else if (next - position > 0.02) {
                    seek(next, true)
                }
            }
            onEnded: paused = true
            onFailed: reason => page.say("This clip cannot be played: " + reason)
        }
        MouseArea { anchors.fill: parent; onClicked: { page.forceActiveFocus(); page.togglePlay() } }
    }

    // The marks of a viewfinder, just outside the picture's corners.
    Item {
        anchors.fill: stage
        anchors.margins: -7
        Brackets {}
    }

    Item {
        id: transport
        height: 56
        anchors.bottom: bottom.top
        anchors.left: parent.left
        anchors.right: side.left
        anchors.leftMargin: 20
        anchors.rightMargin: 20
        // Three groups: the time, the player's buttons, the editing. As
        // the window gets small they are kept from running into each
        // other: the buttons lose their words, the player's outer buttons
        // go (their keys stay), then the second time, then zooming
        // is left to Ctrl and the wheel. The volume's number is only for
        // wide windows.
        readonly property bool tight: width < 1470
        readonly property bool narrow: width < 900
        readonly property bool small: width < 700
        // How many buttons are on each side of play.
        readonly property int flanks: width >= 1180 ? 3 : width >= 1000 ? 2 : width >= 780 ? 1 : 0

        Row {
            id: times
            anchors.verticalCenter: parent.verticalCenter
            spacing: 12
            // Where playback is, as a camera burns the time into a picture.
            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: clock.implicitWidth + 20
                height: 32
                radius: 2
                color: "#000000"
                border.color: Theme.line
                Text {
                    id: clock
                    anchors.centerIn: parent
                    text: Theme.timecode(page.previewing ? page.outputTime(player.position) : player.position)
                    // On black whatever the window's colour: light, always.
                    color: page.previewing ? Theme.accent : "#EEF4F6"
                    font.family: Theme.monoFamily
                    font.pixelSize: 16
                    font.features: { "tnum": 1 }
                }
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: !transport.narrow
                text: {
                    const cut = transport.width >= 1640 && page.edit.changed && Math.abs(page.edit.kept - page.edit.duration) > 0.05
                    if (page.previewing) { return "/  " + Theme.timecode(page.edit.kept || 0) + (cut ? "   of " + Theme.timecode(page.edit.duration) : "") }
                    return "/  " + Theme.timecode(page.edit.duration || 0) + (cut ? "   " + Theme.timecode(page.edit.kept || 0) + " kept" : "")
                }
                color: Theme.textFaint
                font.family: Theme.monoFamily
                font.pixelSize: Theme.body
                font.features: { "tnum": 1 }
            }
        }
        // The player: play in the middle, and on each side of it a frame,
        // a cut, and the start or the end, nearest first. In the middle of
        // the bar while there is room; otherwise between the other two.
        Row {
            id: playing
            anchors.verticalCenter: parent.verticalCenter
            x: Math.round(Math.max(times.width + 12, Math.min((parent.width - width) / 2, editing.x - 12 - width)))
            spacing: 4
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 3
                icon: "restart"
                active: page.previewing
                tip: page.edit.changed ? "Play the result from its start (Enter)" : "Play from the start (Enter)"
                onClicked: { page.forceActiveFocus(); page.playResult() }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 2
                icon: "prevCut"
                tip: "To the cut before (Up)"
                onClicked: { page.forceActiveFocus(); page.toCut(-1) }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 1
                icon: "left"
                tip: "A frame back (Left)"
                onClicked: { page.forceActiveFocus(); player.paused = true; player.step(-1) }
            }
            Item { width: 2; height: 1; visible: transport.flanks >= 1 }
            Rectangle {
                width: 44
                height: 44
                radius: Theme.radius
                color: playArea.containsMouse ? Theme.actionHover : Theme.action
                Accessible.role: Accessible.Button
                Accessible.name: player.paused ? "Play" : "Pause"
                Accessible.onPressAction: page.togglePlay()
                Icon {
                    anchors.centerIn: parent
                    anchors.horizontalCenterOffset: player.paused ? 1 : 0
                    name: player.paused ? "play" : "pause"
                    filled: player.paused
                    size: 20
                    color: Theme.actionInk
                }
                MouseArea { id: playArea; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { page.forceActiveFocus(); page.togglePlay() } }
                Tip { shown: playArea.containsMouse; text: player.paused ? "Play (Space)" : "Pause (Space)" }
            }
            Item { width: 2; height: 1; visible: transport.flanks >= 1 }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 1
                icon: "right"
                tip: "A frame on (Right)"
                onClicked: { page.forceActiveFocus(); player.paused = true; player.step(1) }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 2
                icon: "nextCut"
                tip: "To the cut after (Down)"
                onClicked: { page.forceActiveFocus(); page.toCut(1) }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.flanks >= 3
                icon: "last"
                tip: "To the end (End)"
                onClicked: { page.forceActiveFocus(); player.paused = true; page.go(page.edit.segments[page.edit.segments.length - 1].end - page.frame, true) }
            }
        }
        Row {
            id: editing
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: transport.small ? 4 : 8
            // What is done to the clip, and taking it back, over the
            // tracks it is done to.
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                IconButton { icon: "undo"; tip: "Undo (Ctrl+Z)"; enabled: !!page.edit.canUndo; onClicked: { page.forceActiveFocus(); page.apply(Editor.undo()) } }
                IconButton { icon: "redo"; tip: "Redo (Ctrl+Shift+Z)"; enabled: !!page.edit.canRedo; onClicked: { page.forceActiveFocus(); page.apply(Editor.redo()) } }
                Item { width: 6; height: 1 }
                IconButton { icon: "startHere"; tip: "The part starts at the playhead (Q)"; onClicked: { page.forceActiveFocus(); page.trimHere(true) } }
                IconButton { icon: "endHere"; tip: "The part ends at the playhead (W)"; onClicked: { page.forceActiveFocus(); page.trimHere(false) } }
            }
            Btn {
                anchors.verticalCenter: parent.verticalCenter
                icon: "split"
                text: transport.tight ? "" : "Split"
                tip: "Split at the playhead (S)"
                onClicked: { page.forceActiveFocus(); page.splitHere() }
            }
            Btn {
                anchors.verticalCenter: parent.verticalCenter
                icon: "trash"
                text: transport.tight ? "" : "Remove part"
                tip: "Remove the part chosen, or the one the playhead is in (Delete)"
                enabled: page.edit.segments.length > 1
                onClicked: { page.forceActiveFocus(); page.removeSelected() }
            }
            // Zooming the timeline under this bar.
            Row {
                anchors.verticalCenter: parent.verticalCenter
                visible: !transport.small
                spacing: 2
                Item { width: 6; height: 1 }
                IconButton {
                    icon: "minus"
                    tip: "Zoom the timeline out"
                    enabled: timeline.zoom > 1
                    onClicked: { page.forceActiveFocus(); timeline.zoomTo(timeline.zoom / 1.6, timeline.cx(timeline.position) - timeline.scroll) }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 30
                    horizontalAlignment: Text.AlignHCenter
                    text: "\u00d7" + (timeline.zoom >= 10 ? Math.round(timeline.zoom) : timeline.zoom.toFixed(1))
                    color: timeline.zoom > 1.01 ? Theme.textDim : Theme.textFaint
                    font.family: Theme.monoFamily
                    font.pixelSize: 11
                    font.features: { "tnum": 1 }
                }
                IconButton {
                    icon: "plus"
                    tip: "Zoom the timeline in (Ctrl + wheel)"
                    enabled: timeline.zoom < timeline.maxZoom
                    onClicked: { page.forceActiveFocus(); timeline.zoomTo(timeline.zoom * 1.6, timeline.cx(timeline.position) - timeline.scroll) }
                }
            }
            Item { width: transport.small ? 2 : 8; height: 1 }
            Icon { anchors.verticalCenter: parent.verticalCenter; name: "volume"; size: 16; color: Theme.textFaint }
            Slider {
                id: listening
                anchors.verticalCenter: parent.verticalCenter
                width: transport.width >= 1560 ? 90 : 64
                value: player.volume
                wheelStep: 0.01
                fill: Theme.textDim
                onMoved: v => player.volume = v
                // Kept for the next clip.
                onReleased: Editor.setListening(player.volume)
                Accessible.name: "Playback volume"
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: transport.width >= 1560
                width: 34
                horizontalAlignment: Text.AlignRight
                text: Math.round(player.volume * 100) + "%"
                color: Theme.textFaint
                font.family: Theme.monoFamily
                font.pixelSize: 11
                font.features: { "tnum": 1 }
            }
        }
    }

    Rectangle {
        id: bottom
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: side.left
        height: timeline.implicitHeight + 24
        color: Theme.bg
        Rectangle { width: parent.width; height: 1; color: Theme.line; opacity: 0.6 }

        Timeline {
            id: timeline
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.leftMargin: 20
            anchors.rightMargin: 20
            anchors.topMargin: 12
            height: implicitHeight
            entry: page.entry
            edit: page.edit
            position: player.position
            onScrubbed: (t, exact) => {
                page.forceActiveFocus()
                page.scrubbing = !exact
                page.previewing = false
                page.go(t, exact)
            }
            onHandleGrabbed: page.edgeGrab()
            onHandleMoved: (index, start, t) => page.edgeMove(index, start, t)
            onHandleReleased: page.edgeRelease()
            onVolumeGrabbed: page.volumeGrab()
            onVolumeMoved: (track, volume) => page.volumeMove(track, volume)
            onVolumeReleased: page.volumeRelease()
            onMuteToggled: track => page.toggleMute(track)
            // The keyboard stays on the sync while its keys move it.
            onSyncMoved: ms => { if (!timeline.syncFocused) { page.forceActiveFocus() } page.apply(Editor.setAudioOffset(ms)) }
        }
    }

    // Beside the picture: export in the corner, and under it the clip
    // and the library's others.
    Item {
        id: side
        // A wide window has width the picture cannot use: some goes here.
        // One width or the other, never in between: while the window is
        // being sized only the picture's side gives, and what is in this
        // column is not laid out again at every step.
        width: page.width >= 2000 ? 440 : 340
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        Rectangle { width: 1; height: parent.height; color: Theme.line; opacity: 0.6 }
        ExportPanel {
            id: exporting
            width: parent.width
            revision: page.revision
            clipId: page.entry.id
            title: page.title
            onRenamed: name => page.rename(name)
            onEntered: page.forceActiveFocus()
            onSay: message => page.say(message)
        }
        ToolPanel {
            id: toolPanel
            anchors.top: exporting.bottom
            anchors.bottom: parent.bottom
            width: parent.width
            entry: page.entry
            edit: page.edit
            onClipWanted: clip => page.openClip(clip)
            onFrameWanted: page.saveFrame()
            onSay: message => page.say(message)
        }
    }
}

import QtQuick
import Zenith

// The clip laid out in time: its frames, its audio tracks, the stretches
// that are kept, and where playback is. Times are seconds of the clip.
Item {
    id: timeline
    required property var entry
    // The editor's state: {duration, segments, gains}.
    property var edit: ({ duration: 1, segments: [], gains: [] })
    property real position: 0
    property int selected: -1
    // A handle is held: the view stays where it is under the pointer.
    property bool grabbing: false

    signal scrubbed(real t, bool exact)
    signal handleGrabbed()
    signal handleMoved(int index, bool start, real t)
    signal handleReleased()
    signal volumeGrabbed()
    signal volumeMoved(int track, real volume)
    signal volumeReleased()
    signal muteToggled(int track)
    // The sound is to be put this many milliseconds later (below zero:
    // earlier) against the picture.
    signal syncMoved(int ms)
    // How much later (below zero: earlier) the sound is put against the
    // picture, in milliseconds: the waveforms are drawn where it is heard.
    readonly property int sync: edit.audioOffsetMs || 0
    // The furthest the sound can be moved, either way.
    readonly property int syncMost: 3000
    function syncBy(ms) { syncMoved(Math.max(-syncMost, Math.min(syncMost, sync + ms))) }
    // The keyboard is on the sync: it stays there while its keys move it.
    readonly property bool syncFocused: syncValue.activeFocus

    readonly property int gutter: 168
    readonly property int filmHeight: 56
    readonly property int trackHeight: 40
    // The part of the width that shows time, and how far it is zoomed in:
    // at 1 the whole clip fits, and `scroll` is how far the view has moved.
    readonly property real span: Math.max(1, width - gutter)
    property real zoom: 1
    property real scroll: 0
    readonly property real maxZoom: Math.max(1, duration / 2)
    readonly property real content: span * zoom
    readonly property real duration: Math.max(0.1, edit.duration || entry.seconds)
    // How many frames the filmstrip has: a few fixed sizes, so zooming
    // asks for a handful of strips and not one for every step.
    readonly property int tiles: {
        const fit = content / (filmHeight * 16 / 9)
        for (const n of [8, 16, 32, 64]) {
            if (n >= fit) { return n }
        }
        return 120
    }
    // What is cut away: the stretches between the kept ones.
    readonly property var gaps: {
        const gaps = []
        let at = 0
        for (const s of edit.segments) {
            if (s.start - at > 0.0005) { gaps.push({ start: at, end: s.start }) }
            at = s.end
        }
        if (duration - at > 0.0005) { gaps.push({ start: at, end: duration }) }
        return gaps
    }
    property string film: ""
    property var waves: ({})

    implicitHeight: 22 + filmHeight + 8 + entry.tracks.length * (trackHeight + 6)

    // A time as a distance along the zoomed content, and back from a
    // position in the visible part.
    function cx(t) { return t / duration * content }
    function timeAt(viewX) { return Math.max(0, Math.min(duration, (viewX + scroll) / content * duration)) }

    // Zoom to `next`, keeping the time under `viewX` where it is.
    function zoomTo(next, viewX) {
        const at = timeAt(viewX)
        zoom = Math.max(1, Math.min(maxZoom, next))
        scroll = Math.max(0, Math.min(content - span, cx(at) - viewX))
    }
    function scrollBy(dx) { scroll = Math.max(0, Math.min(content - span, scroll + dx)) }
    // Keep a time in view, as playback moves on.
    function reveal(t) {
        const at = cx(t)
        if (at < scroll + 8 || at > scroll + span - 8) {
            scroll = Math.max(0, Math.min(content - span, at - span * 0.15))
        }
    }
    onPositionChanged: if (zoom > 1 && !scrubArea.pressed && !grabbing) { reveal(position) }
    onSpanChanged: scrollBy(0)

    // The strip for this zoom: shown at once when it is cached; otherwise
    // the one already there stays until the new one is made.
    function wantFilm() {
        const cached = Media.filmstrip(entry.path, entry.seconds, tiles)
        if (cached !== "") { film = cached }
    }
    function load() {
        wantFilm()
        for (let i = 0; i < entry.tracks.length; i++) {
            Media.waveform(entry.path, i, buckets)
        }
    }
    // How finely a track's shape is asked for: by the clip's length, so a
    // longer clip zoomed in is not a few wide steps. Zoomed in the furthest
    // two seconds fill the view, some 330 bars: that many for every two
    // seconds, in round hundreds (each count is a file in the cache), no
    // fewer than a short clip always had and no more than `Media` makes.
    readonly property int buckets: Math.max(800, Math.min(8000, Math.ceil(entry.seconds * 1.65) * 100))
    Component.onCompleted: load()
    // Zooming passes through sizes on its way: only where it rests is asked for.
    onTilesChanged: filmWait.restart()
    Timer { id: filmWait; interval: 250; onTriggered: timeline.wantFilm() }

    Connections {
        target: Media
        function onFilmstripReady(path, count, file) {
            if (path === timeline.entry.path && count === timeline.tiles) { timeline.film = file }
        }
        function onWaveformReady(path, track, peaks) {
            if (path === timeline.entry.path) {
                const next = Object.assign({}, timeline.waves)
                next[track] = JSON.parse(peaks)
                timeline.waves = next
            }
        }
    }

    // Beside the picture's row, the audio synchronization, for this clip
    // alone: the sound a step later or earlier against the picture.
    Item {
        y: 22
        width: timeline.gutter
        height: timeline.filmHeight
        Row {
            x: 4
            anchors.verticalCenter: parent.verticalCenter
            height: 26
            spacing: 2
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: 40
                text: "Sync"
                color: Theme.textDim
                font.family: Theme.family
                font.pixelSize: Theme.small
                font.weight: Font.DemiBold
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                implicitWidth: 26
                implicitHeight: 26
                iconSize: 14
                icon: "minus"
                tip: "Sound 100 ms earlier"
                enabled: timeline.sync > -timeline.syncMost
                onClicked: timeline.syncBy(-100)
            }
            Rectangle {
                id: syncValue
                anchors.verticalCenter: parent.verticalCenter
                width: 64
                height: 26
                radius: 2
                color: syncArea.containsMouse && timeline.sync !== 0 ? Theme.raised : "transparent"
                // Reachable with Tab: the arrow keys move the sound a step
                // (a big one with Shift), Delete or Backspace puts it back.
                activeFocusOnTab: true
                Keys.onPressed: event => {
                    const by = event.modifiers & Qt.ShiftModifier ? 100 : 10
                    if (event.key === Qt.Key_Left) { timeline.syncBy(-by) }
                    else if (event.key === Qt.Key_Right) { timeline.syncBy(by) }
                    else if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) { if (timeline.sync !== 0) { timeline.syncMoved(0) } }
                    else { return }
                    event.accepted = true
                }
                Accessible.role: Accessible.Slider
                Accessible.name: "Audio synchronization, " + timeline.sync + " milliseconds"
                Accessible.onIncreaseAction: timeline.syncBy(10)
                Accessible.onDecreaseAction: timeline.syncBy(-10)
                FocusRing {}
                Text {
                    anchors.centerIn: parent
                    text: timeline.sync === 0 ? "0 ms" : (timeline.sync > 0 ? "+" : "\u2212") + Math.abs(timeline.sync) + " ms"
                    color: timeline.sync === 0 ? Theme.textFaint : Theme.accentWords
                    font.family: Theme.monoFamily
                    font.pixelSize: 12
                    font.features: { "tnum": 1 }
                }
                MouseArea {
                    id: syncArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: timeline.sync !== 0 ? Qt.PointingHandCursor : Qt.ArrowCursor
                    onClicked: if (timeline.sync !== 0) { timeline.syncMoved(0) }
                    // The wheel is the fine step: 10 ms a notch.
                    onWheel: wheel => {
                        if (wheel.angleDelta.y !== 0) { timeline.syncBy(wheel.angleDelta.y > 0 ? 10 : -10) }
                    }
                }
                Tip {
                    shown: syncArea.containsMouse
                    text: timeline.sync === 0 ? "Audio synchronization: this clip's sound is where it was recorded. The wheel moves it 10 ms at a time"
                        : "This clip's sound is " + Math.abs(timeline.sync) + " ms " + (timeline.sync > 0 ? "later" : "earlier") + ". Click to put it back"
                }
            }
            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                implicitWidth: 26
                implicitHeight: 26
                iconSize: 14
                icon: "plus"
                tip: "Sound 100 ms later"
                enabled: timeline.sync < timeline.syncMost
                onClicked: timeline.syncBy(100)
            }
        }
    }

    // The audio tracks' names, mute and volume, beside their waveforms.
    Column {
        id: labels
        y: 22 + timeline.filmHeight + 8
        width: timeline.gutter
        spacing: 6
        Repeater {
            model: timeline.entry.tracks
            Item {
                id: row
                required property int index
                required property string modelData
                readonly property var gain: timeline.edit.gains[index] || { volume: 1, muted: false }
                width: labels.width
                height: timeline.trackHeight

                IconButton {
                    id: mute
                    anchors.verticalCenter: parent.verticalCenter
                    implicitWidth: 28
                    implicitHeight: 28
                    iconSize: 15
                    icon: row.gain.muted ? "mute" : "volume"
                    tint: row.gain.muted ? Theme.record : Theme.textDim
                    tip: row.gain.muted ? "Unmute" : "Mute"
                    onClicked: timeline.muteToggled(row.index)
                }
                Column {
                    anchors.left: mute.right
                    anchors.leftMargin: 6
                    anchors.verticalCenter: parent.verticalCenter
                    width: timeline.gutter - 28 - 6 - 16
                    spacing: 1
                    Item {
                        width: parent.width
                        height: trackName.implicitHeight
                        Text {
                            id: trackName
                            width: parent.width - percent.width - 6
                            text: row.modelData
                            elide: Text.ElideRight
                            color: row.gain.muted ? Theme.textFaint : Theme.textDim
                            font.family: Theme.family
                            font.pixelSize: Theme.small
                            font.weight: Font.DemiBold
                        }
                        // How loud the track is set: 100% is as recorded.
                        Text {
                            id: percent
                            anchors.right: parent.right
                            anchors.baseline: trackName.baseline
                            text: row.gain.muted ? "muted" : Math.round(row.gain.volume * 100) + "%"
                            color: row.gain.muted ? Theme.record : Math.abs(row.gain.volume - 1) < 0.005 ? Theme.textFaint : Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: 11
                            font.features: { "tnum": 1 }
                        }
                    }
                    Slider {
                        id: level
                        width: parent.width
                        implicitHeight: 14
                        from: 0
                        to: 2
                        // A notch of the wheel is 1%.
                        wheelStep: 0.01
                        value: row.gain.volume
                        opacity: row.gain.muted ? 0.4 : 1
                        fill: Theme.textDim
                        onPressed: timeline.volumeGrabbed()
                        // Dragged, it rests at 100%; the wheel's small steps pass it by.
                        onMoved: v => timeline.volumeMoved(row.index, !level.fine && Math.abs(v - 1) < 0.06 ? 1 : v)
                        onReleased: timeline.volumeReleased()
                        Accessible.name: row.modelData + " volume"
                    }
                }
            }
        }
    }

    // The part that shows time. What is in it is as wide as the zoom makes
    // it, and slides under this window onto it.
    Item {
        id: view
        x: timeline.gutter
        width: timeline.span
        height: parent.height
        clip: true

        // Scrubbing, scrolling and zooming: press or drag to move through
        // the clip, the wheel to scroll, Ctrl and the wheel to zoom.
        MouseArea {
            id: scrubArea
            anchors.fill: parent
            onPressed: mouse => timeline.scrubbed(timeline.timeAt(mouse.x), false)
            onPositionChanged: mouse => { if (pressed) { timeline.scrubbed(timeline.timeAt(mouse.x), false) } }
            onReleased: mouse => timeline.scrubbed(timeline.timeAt(mouse.x), true)
            onDoubleClicked: mouse => {
                const at = timeline.timeAt(mouse.x)
                timeline.selected = timeline.edit.segments.findIndex(s => at >= s.start && at < s.end)
            }
            onWheel: wheel => {
                const turn = wheel.angleDelta.y !== 0 ? wheel.angleDelta.y : wheel.angleDelta.x
                if (wheel.modifiers & Qt.ControlModifier) {
                    timeline.zoomTo(timeline.zoom * Math.pow(1.0015, turn), wheel.x)
                } else {
                    timeline.scrollBy(-turn)
                }
            }
        }

        Item {
            id: strip
            x: -timeline.scroll
            width: timeline.content
            height: parent.height

            // The ruler: a mark every so often, with its time.
            Item {
                id: ruler
                width: parent.width
                height: 22
                readonly property real every: {
                    const want = timeline.duration / Math.max(1, width / 90)
                    for (const s of [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300]) {
                        if (s >= want) { return s }
                    }
                    return 600
                }
                // Only the marks in view are made, however far it is zoomed in.
                readonly property int first: Math.max(0, Math.floor(timeline.scroll / timeline.content * timeline.duration / every) - 1)
                readonly property int count: Math.max(0, Math.min(
                    Math.floor(timeline.duration / every) + 1 - first,
                    Math.ceil(timeline.span / Math.max(1, timeline.cx(every))) + 3))
                Repeater {
                    model: ruler.count
                    Item {
                        required property int index
                        readonly property real at: (ruler.first + index) * ruler.every
                        x: timeline.cx(at)
                        Rectangle { width: 1; height: 5; y: 16; color: Theme.textFaint }
                        Text {
                            x: 4
                            y: 2
                            text: ruler.every < 1 ? Theme.timecode(parent.at) : Theme.duration(parent.at)
                            color: Theme.textFaint
                            font.family: Theme.monoFamily
                            font.pixelSize: 11
                            font.features: { "tnum": 1 }
                        }
                    }
                }
            }

            Rectangle {
                y: ruler.height
                width: parent.width
                height: timeline.filmHeight
                radius: Theme.radius
                color: Theme.surface
                clip: true
                Image {
                    anchors.fill: parent
                    source: Library.fileUrl(timeline.film)
                    // The longest strip is 120 frames of 160 across: wider
                    // than many graphics cards take a picture (16384). Read
                    // no wider than they do, its frames are still taller
                    // than the row they are shown in. (A shorter strip is
                    // read as it is: this only ever makes one smaller.)
                    sourceSize.width: 16384
                    fillMode: Image.Stretch
                    asynchronous: true
                    opacity: status === Image.Ready ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: Theme.smooth } }
                }
            }

            Column {
                y: ruler.height + timeline.filmHeight + 8
                width: parent.width
                spacing: 6
                Repeater {
                    model: timeline.entry.tracks
                    Rectangle {
                        id: lane
                        required property int index
                        readonly property var gain: timeline.edit.gains[index] || { volume: 1, muted: false }
                        width: parent.width
                        height: timeline.trackHeight
                        radius: Theme.radius
                        color: Theme.surface
                        // As wide as the view and kept under it: zoomed
                        // in, only what can be seen is drawn.
                        Waveform {
                            x: timeline.scroll + 1
                            width: timeline.span - 2
                            height: parent.height
                            total: timeline.content - 2
                            // Drawn where it will be heard: later or
                            // earlier by the clip's sync.
                            offset: timeline.scroll - timeline.cx(timeline.sync / 1000)
                            peaks: timeline.waves[lane.index] || []
                            gain: lane.gain.muted ? 0.15 : lane.gain.volume
                            tint: lane.gain.muted ? Theme.line : Theme.onWindow([Theme.accent, "#F2D06B", "#B79CFF"][lane.index % 3])
                        }
                    }
                }
            }

            // What is cut away, darkened across every row.
            // By count, as the kept stretches below, so a drag moves them
                // and does not make them again.
            Repeater {
                model: timeline.gaps.length
                Rectangle {
                    required property int index
                    readonly property var gap: timeline.gaps[index] || { start: 0, end: 0 }
                    x: timeline.cx(gap.start)
                    y: ruler.height
                    width: timeline.cx(gap.end) - x
                    height: strip.height - ruler.height
                    color: "#C80C0D10"
                }
            }

            // The kept stretches: a frame around each, with a handle at both ends.
            Repeater {
                // By count, not by the list: the list is replaced on every
                // step of a drag, and a handle must outlive that to keep
                // hold of the pointer.
                model: timeline.edit.segments.length
                Item {
                    id: piece
                    required property int index
                    readonly property var seg: timeline.edit.segments[index] || { start: 0, end: 0 }
                    readonly property bool chosen: timeline.selected === index
                    // Too narrow for two handles inside it: they sit outside,
                    // so both can still be taken hold of.
                    readonly property bool narrow: width < 30
                    x: timeline.cx(seg.start)
                    y: ruler.height - 2
                    width: Math.max(4, timeline.cx(seg.end) - x)
                    height: strip.height - ruler.height + 2

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.radius
                        color: "transparent"
                        border.width: 2
                        border.color: piece.chosen ? Theme.text : Theme.accent
                    }
                    Repeater {
                        model: [true, false]
                        Rectangle {
                            id: handle
                            required property bool modelData
                            x: modelData ? (piece.narrow ? -width : 0) : (piece.narrow ? piece.width : piece.width - width)
                            width: 10
                            height: piece.height
                            radius: 2
                            // To be taken hold of, so in the colour of things to do.
                            color: grip.pressed || grip.containsMouse ? Theme.actionHover : Theme.action
                            Rectangle { anchors.centerIn: parent; width: 2; height: 18; radius: 1; color: Theme.actionInk; opacity: 0.7 }
                            MouseArea {
                                id: grip
                                anchors.fill: parent
                                anchors.margins: -5
                                // Outside a narrow part the two do not reach over each other.
                                anchors.rightMargin: piece.narrow && handle.modelData ? 0 : -5
                                anchors.leftMargin: piece.narrow && !handle.modelData ? 0 : -5
                                hoverEnabled: true
                                cursorShape: Qt.SizeHorCursor
                                preventStealing: true
                                // How far the pointer was from the edge the
                                // handle moves (the part's start or end) when
                                // it took hold: the edge keeps that distance,
                                // so it does not jump to the pointer at the
                                // first move.
                                property real held: 0
                                onPressed: mouse => {
                                    const edge = timeline.cx(handle.modelData ? piece.seg.start : piece.seg.end) - timeline.scroll
                                    held = mapToItem(view, mouse.x, 0).x - edge
                                    timeline.selected = piece.index
                                    timeline.grabbing = true
                                    timeline.handleGrabbed()
                                }
                                onPositionChanged: mouse => {
                                    if (pressed) {
                                        const p = mapToItem(view, mouse.x, 0)
                                        timeline.handleMoved(piece.index, handle.modelData, timeline.timeAt(p.x - held))
                                    }
                                }
                                onReleased: { timeline.grabbing = false; timeline.handleReleased() }
                                onCanceled: { timeline.grabbing = false; timeline.handleReleased() }
                            }
                        }
                    }
                }
            }

            // Where playback is.
            Item {
                x: timeline.cx(timeline.position) - 1
                y: 14
                width: 2
                height: strip.height - 14
                Rectangle { anchors.fill: parent; color: Theme.text }
                Rectangle { width: 12; height: 12; radius: 3; x: -5; y: -4; rotation: 45; color: Theme.text }
            }
        }
    }

    // Zoomed in: where the view is within the whole clip.
    Rectangle {
        visible: timeline.zoom > 1.01
        x: timeline.gutter
        y: -9
        width: timeline.span
        height: 4
        radius: 2
        color: Theme.surface
        Rectangle {
            x: timeline.scroll / timeline.content * parent.width
            width: Math.max(16, parent.width / timeline.zoom)
            height: parent.height
            radius: 2
            color: Theme.textFaint
        }
    }
}

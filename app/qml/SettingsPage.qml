import QtQuick
import QtQuick.Dialogs
import Zenith

// The settings. A change is kept and reaches the recorder as it is made.
FocusScope {
    id: page
    signal closed()
    signal say(string message)

    property var cfg: JSON.parse(Settings.load())
    // The monitors and codecs arrive a moment after the page opens.
    property var machine: JSON.parse(Settings.machine())
    readonly property var estimate: { machine; return JSON.parse(Settings.estimate(JSON.stringify(cfg))) }
    Connections {
        target: Settings
        function onMachineReady() { page.machine = JSON.parse(Settings.machine()) }
        function onReloadFailed(message) { page.say(message) }
        function onReportCopied() { page.say("Report copied. Paste it where you are getting help.") }
    }
    // Voice chat has a track of its own: the game sound is then picked by
    // app, not by speaker.
    readonly property bool splitChat: cfg.audio.chat && cfg.audio.voice_apps.length > 0
    // A list of sound devices, with the chosen one in it even when it is
    // not plugged in now (the default one is recorded until it is back).
    function withChosen(devices, chosen) {
        if (devices.some(d => d.name === chosen)) { return devices }
        return devices.concat([{ name: chosen, label: "Not connected now: " + chosen + " (the default is recorded meanwhile)" }])
    }
    // What the recorder says about the clip key once it has tried to take
    // it: another program may hold the combination already.
    property string keyTrouble: ""
    // On Linux the desktop holds the key for the recorder: the one it gave,
    // and why there is none when it gave none.
    property string keyHeld: ""
    property string keyMissing: ""
    Timer { id: keyCheck; interval: 1500; onTriggered: Recorder.poll() }
    Connections {
        target: Recorder
        function onStatusReady(status) {
            const s = JSON.parse(status)
            page.keyTrouble = s.detail && /hotkey|key/i.test(s.detail) && s.state !== "failed" ? s.detail : ""
            page.keyHeld = s.hotkey || ""
            page.keyMissing = s.hotkeyMissing || ""
        }
    }
    // How things stand with new versions.
    property var up: JSON.parse(Updates.status())
    // Zenith's page, where its versions are put out.
    readonly property string github: "https://github.com/Draysh/zenith-releases"
    Connections {
        target: Updates
        function onChanged() { page.up = JSON.parse(Updates.status()) }
    }
    // The patch notes of this version are folded away until asked for.
    // (`--notes` opens the page with them out, for a look without a person.)
    property bool showNotes: Qt.application.arguments.indexOf("--notes") >= 0
    // Leaving: a field still being typed in is kept first.
    function leave() {
        page.forceActiveFocus()
        page.closed()
    }

    // Change one value: `set("capture", "fps", 60)`.
    function set(section, key, value) {
        const next = JSON.parse(JSON.stringify(cfg))
        next[section][key] = value
        cfg = next
        saver.restart()
    }
    function save() {
        const kept = JSON.parse(Settings.save(JSON.stringify(cfg)))
        if (kept.error) {
            page.say(kept.error)
        }
        if (kept.capture) {
            delete kept.error
            cfg = kept
        }
    }
    // The colours are seen as they are picked.
    onCfgChanged: Theme.use(cfg.look)
    // What each quality means, and what a minute of it comes to on this
    // monitor at the size and frame rate chosen.
    readonly property var qualities: {
        machine
        const words = {
            low: "Small files. Fast motion and fine detail break up into blocks.",
            medium: "Good for sharing. Fast motion looks a little soft.",
            high: "Sharp in almost every game. The usual choice.",
            ultra: "As close to the game as it gets. Hard to tell from High unless a lot moves at once."
        }
        return ["low", "medium", "high", "ultra"].map(key => {
            const trial = JSON.parse(JSON.stringify(cfg))
            trial.capture.quality = key
            const e = JSON.parse(Settings.estimate(JSON.stringify(trial)))
            return {
                key: key,
                label: key.charAt(0).toUpperCase() + key.slice(1),
                words: words[key],
                megabytes: ((e.kbps || 0) + 128 * (e.tracks || 0)) * 60 / 8 / 1000
            }
        })
    }
    function lengthWords(seconds) {
        if (seconds < 60) { return seconds + " seconds" }
        const m = Math.floor(seconds / 60)
        const s = seconds % 60
        return m + (m === 1 ? " minute" : " minutes") + (s ? " " + s + " s" : "")
    }
    function sizeOptions() {
        const options = [
            { key: "native", label: "Native" },
            { key: "2560x1440", label: "1440p" },
            { key: "1920x1080", label: "1080p" },
            { key: "1280x720", label: "720p" }
        ]
        if (!options.some(o => o.key === cfg.capture.resolution)) {
            options.push({ key: cfg.capture.resolution, label: cfg.capture.resolution })
        }
        return options
    }

    FolderDialog {
        id: folderDialog
        title: "Clip folder"
        currentFolder: Library.fileUrl(page.cfg.library.dir)
        onAccepted: page.set("library", "dir", Library.localPath(selectedFolder.toString()))
    }

    focus: true
    Keys.onEscapePressed: page.leave()
    Component.onDestruction: flush()
    // A change still waiting to be saved is saved now: the page or the
    // whole window is going.
    function flush() {
        if (saver.running) { saver.stop(); save() }
    }
    // A dragged slider is one save, when it comes to rest.
    Timer { id: saver; interval: 500; onTriggered: page.save() }

    Item {
        id: header
        // Everything under the sheet waits for it: no key or Tab reaches it.
        enabled: !addGame.open
        height: 56
        width: parent.width
        IconButton {
            id: back
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            icon: "back"
            tip: "Back (Esc)"
            onClicked: page.leave()
        }
        Text {
            anchors.left: back.right
            anchors.leftMargin: 10
            anchors.verticalCenter: parent.verticalCenter
            text: "Settings"
            color: Theme.accentWords
            font.family: Theme.titleFamily
            font.pixelSize: 26
            font.capitalization: Font.AllUppercase
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line; opacity: 0.6 }
    }

    Flickable {
        id: scroller
        enabled: !addGame.open
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        contentHeight: body.implicitHeight + 64
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        // A notch of the mouse wheel is a good step down the page, not
        // the few lines a list gives it by itself.
        WheelHandler {
            onWheel: event => {
                const end = Math.max(0, scroller.contentHeight - scroller.height)
                if (event.pixelDelta.y !== 0) {
                    glide.stop()
                    scroller.contentY = Math.max(0, Math.min(end, scroller.contentY - event.pixelDelta.y))
                    return
                }
                glide.to = Math.max(0, Math.min(end, (glide.running ? glide.to : scroller.contentY) - event.angleDelta.y * 1.5))
                glide.restart()
            }
        }
        NumberAnimation { id: glide; target: scroller; property: "contentY"; duration: 160; easing.type: Easing.OutCubic }
        // `--scroll 600` opens the page that far down, for a look at what
        // is below the first screenful without a person.
        Timer {
            readonly property int at: Qt.application.arguments.indexOf("--scroll")
            interval: 300
            running: at >= 0
            onTriggered: scroller.contentY = Math.max(0, Math.min(scroller.contentHeight - scroller.height, parseInt(Qt.application.arguments[at + 1]) || 0))
        }

        // In a wide window the groups stand in two columns side by side;
        // in a narrow one the second goes under the first.
        Flow {
            id: body
            readonly property bool wide: scroller.width >= 1180
            readonly property real column: wide ? Math.floor((width - spacing) / 2) : width
            y: 28
            width: Math.min(wide ? 1560 : 760, scroller.width - 48)
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 28

            Rectangle {
                visible: !page.machine.recorder
                width: body.width
                height: missing.implicitHeight + 28
                radius: Theme.radiusLarge
                color: "#33FF5A52"
                border.color: Theme.record
                Text {
                    id: missing
                    anchors.fill: parent
                    anchors.margins: 14
                    wrapMode: Text.WordWrap
                    text: Qt.platform.os === "windows"
                        ? "Zenith's recorder could not be asked what this computer offers, so nothing can be captured. Installing Zenith again puts it back."
                        : "gpu-screen-recorder was not found, so nothing can be captured. Install it from your distribution's packages, then open this page again."
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.body
                }
            }

            Column {
            width: body.column
            spacing: 28

            SettingGroup {
                title: "Clips"
                SettingRow {
                    label: "Clip length"
                    note: "How far back a clip reaches when you press the shortcut. Kept in memory: about "
                        + Theme.size((page.estimate.megabytes || 0) * 1024 * 1024) + " while capturing."
                    stacked: true
                    Row {
                        spacing: 16
                        width: parent.width
                        Slider {
                            id: lengthSlider
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - 150
                            from: 15
                            to: 600
                            value: page.cfg.capture.buffer_secs
                            // Steps of 5 s up to a minute, then of 15.
                            onMoved: v => page.set("capture", "buffer_secs", v <= 60 ? Math.round(v / 5) * 5 : Math.round(v / 15) * 15)
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 134
                            horizontalAlignment: Text.AlignRight
                            text: page.lengthWords(page.cfg.capture.buffer_secs)
                            color: Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.body
                        }
                    }
                }
                SettingRow {
                    label: "Capture"
                    note: page.cfg.capture.activation === "game"
                        ? "Only while a game is running, so nothing is recorded on the desktop."
                        : "All the time, whatever is on screen."
                    Segmented {
                        options: [{ key: "game", label: "During games" }, { key: "always", label: "Always" }]
                        value: page.cfg.capture.activation
                        onChosen: key => page.set("capture", "activation", key)
                    }
                }
                SettingRow {
                    label: "Clip folder"
                    note: "A folder for each game is made inside it."
                    Row {
                        spacing: 6
                        Field {
                            implicitWidth: 300
                            value: page.cfg.library.dir
                            onCommitted: text => { if (text.trim() !== "" && text !== page.cfg.library.dir) { page.set("library", "dir", text.trim()) } }
                        }
                        IconButton {
                            anchors.verticalCenter: parent.verticalCenter
                            icon: "folder"
                            tip: "Choose a folder"
                            onClicked: folderDialog.open()
                        }
                    }
                }
            }

            SettingGroup {
                title: "Video"
                SettingRow {
                    label: "Resolution"
                    note: "Recording at " + (page.estimate.width || "?") + "×" + (page.estimate.height || "?")
                        + ", about " + ((page.estimate.kbps || 0) / 1000).toFixed(1) + " Mbit/s."
                    Segmented {
                        options: page.sizeOptions()
                        value: page.cfg.capture.resolution
                        onChosen: key => page.set("capture", "resolution", key)
                    }
                }
                SettingRow {
                    label: "Frame rate"
                    Segmented {
                        options: [30, 60, 120].concat([30, 60, 120].indexOf(page.cfg.capture.fps) < 0 ? [page.cfg.capture.fps] : [])
                            .map(f => ({ key: String(f), label: f + " fps" }))
                        value: String(page.cfg.capture.fps)
                        onChosen: key => page.set("capture", "fps", parseInt(key))
                    }
                }
                SettingRow {
                    label: "Quality"
                    note: "How much of the picture is kept. Higher is sharper when a lot moves at once, and makes larger clips that take more memory while capturing."
                    stacked: true
                    Column {
                        width: parent.width
                        spacing: 12
                        Segmented {
                            options: page.qualities.map(q => ({ key: q.key, label: q.label }))
                            value: page.cfg.capture.quality
                            onChosen: key => page.set("capture", "quality", key)
                        }
                        Column {
                            width: parent.width
                            spacing: 7
                            Repeater {
                                model: page.qualities
                                Item {
                                    id: meaning
                                    required property var modelData
                                    readonly property bool on: page.cfg.capture.quality === modelData.key
                                    width: parent.width
                                    height: Math.max(meant.implicitHeight, 16)
                                    Text {
                                        width: 64
                                        text: meaning.modelData.label
                                        color: meaning.on ? Theme.accentWords : Theme.textDim
                                        font.family: Theme.family
                                        font.pixelSize: Theme.small
                                        font.weight: Font.DemiBold
                                    }
                                    Text {
                                        id: meant
                                        x: 72
                                        width: parent.width - 72 - 150
                                        text: meaning.modelData.words
                                        wrapMode: Text.WordWrap
                                        color: meaning.on ? Theme.text : Theme.textFaint
                                        font.family: Theme.family
                                        font.pixelSize: Theme.small
                                        lineHeight: 1.25
                                    }
                                    Text {
                                        anchors.right: parent.right
                                        text: "about " + Theme.size(meaning.modelData.megabytes * 1024 * 1024) + " a minute"
                                        color: meaning.on ? Theme.text : Theme.textFaint
                                        font.family: Theme.monoFamily
                                        font.pixelSize: Theme.small
                                    }
                                }
                            }
                        }
                    }
                }
                SettingRow {
                    label: "Format"
                    note: page.cfg.capture.codec === "h264"
                        ? "H.264 plays everywhere, also when a clip is shared as it is."
                        : "Smaller for the same picture, but some apps cannot play it. Exports are always H.264."
                    Segmented {
                        options: [
                            { key: "h264", label: "H.264" }, { key: "hevc", label: "HEVC" }, { key: "av1", label: "AV1" }
                        ].filter(c => page.machine.codecs.indexOf(c.key) >= 0 || c.key === page.cfg.capture.codec || c.key === "h264")
                        value: page.cfg.capture.codec
                        onChosen: key => page.set("capture", "codec", key)
                    }
                }
                SettingRow {
                    visible: page.machine.monitors.length > 1
                    label: "Monitor"
                    Segmented {
                        options: [{ key: "screen", label: "Largest" }].concat(
                            page.machine.monitors.map(m => ({ key: m.name, label: m.name + "  " + m.width + "×" + m.height })))
                        value: page.cfg.capture.source
                        onChosen: key => page.set("capture", "source", key)
                    }
                }
                SettingRow {
                    label: "Show the mouse pointer"
                    Toggle { on: page.cfg.capture.cursor; onToggled: on => page.set("capture", "cursor", on) }
                }
            }

            SettingGroup {
                title: "Sound"
                SettingRow {
                    label: "Game sound"
                    note: page.splitChat ? "Everything that plays except the voice chat apps, whichever speakers it plays on."
                                         : "Everything that plays, on its own track."
                    Toggle { on: page.cfg.audio.game; onToggled: on => page.set("audio", "game", on) }
                }
                SettingRow {
                    visible: page.cfg.audio.game && !page.splitChat && page.machine.speakers.length > 0
                    height: visible ? implicitHeight : 0
                    label: "Record what plays on"
                    note: "The speakers or headphones whose sound goes into the clip."
                    stacked: true
                    Choices {
                        width: parent.width
                        options: page.withChosen(page.machine.speakers, page.cfg.audio.output_device)
                        value: page.cfg.audio.output_device
                        onChosen: name => page.set("audio", "output_device", name)
                    }
                }
                SettingRow {
                    label: "Voice chat"
                    note: "The apps below are kept out of the game sound and get a track of their own."
                    Toggle { on: page.cfg.audio.chat; onToggled: on => page.set("audio", "chat", on) }
                }
                SettingRow {
                    visible: page.cfg.audio.chat
                    label: "Voice chat apps"
                    note: "Names as the sound system shows them, separated by commas."
                    stacked: true
                    Field {
                        width: parent.width
                        value: page.cfg.audio.voice_apps.join(", ")
                        onCommitted: text => {
                            const apps = text.split(",").map(a => a.trim()).filter(a => a !== "")
                            if (apps.join("|") !== page.cfg.audio.voice_apps.join("|")) { page.set("audio", "voice_apps", apps) }
                        }
                    }
                }
                SettingRow {
                    visible: Qt.platform.os === "windows"
                    height: visible ? implicitHeight : 0
                    label: "Audio synchronization"
                    note: "If the sound in your clips comes before or after what you see, move it here: every clip saved from now on has it. A clip you already have is moved by itself in the editor (Sync, beside its timeline). Most computers need none."
                    Row {
                        spacing: 12
                        Slider {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 200
                            from: -500
                            to: 500
                            value: page.cfg.audio.offset_ms || 0
                            // Steps of ten milliseconds; a little either side of none is none.
                            onMoved: v => page.set("audio", "offset_ms", Math.abs(v) < 15 ? 0 : Math.round(v / 10) * 10)
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 120
                            horizontalAlignment: Text.AlignRight
                            text: !page.cfg.audio.offset_ms ? "none" : Math.abs(page.cfg.audio.offset_ms) + " ms " + (page.cfg.audio.offset_ms > 0 ? "later" : "earlier")
                            color: Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.body
                        }
                    }
                }
                SettingRow {
                    label: "Microphone"
                    note: "Your voice, on its own track."
                    Toggle { on: page.cfg.audio.microphone; onToggled: on => page.set("audio", "microphone", on) }
                }
                SettingRow {
                    visible: page.cfg.audio.microphone && page.machine.microphones.length > 0
                    height: visible ? implicitHeight : 0
                    label: "Microphone to record"
                    stacked: true
                    Choices {
                        width: parent.width
                        options: page.withChosen(page.machine.microphones, page.cfg.audio.input_device)
                        value: page.cfg.audio.input_device
                        onChosen: name => page.set("audio", "input_device", name)
                    }
                }
            }

            SettingGroup {
                title: "Editing"
                SettingRow {
                    label: "Initial volume"
                    note: "How loud each track is when a clip is opened in the editor for the first time. From then on a clip keeps its own volumes: changing one clip does not change another."
                    stacked: true
                    Column {
                        width: parent.width
                        spacing: 4
                        Repeater {
                            model: [
                                { key: "game_volume", label: "Game" },
                                { key: "chat_volume", label: "Voice chat" },
                                { key: "microphone_volume", label: "Microphone" }
                            ]
                            Item {
                                id: starting
                                required property var modelData
                                readonly property real volume: page.cfg.editor ? page.cfg.editor[modelData.key] : 1
                                width: parent.width
                                height: 30
                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 104
                                    text: starting.modelData.label
                                    color: Theme.textDim
                                    font.family: Theme.family
                                    font.pixelSize: Theme.body
                                }
                                Slider {
                                    x: 112
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: parent.width - 112 - 64
                                    from: 0
                                    to: 2
                                    value: starting.volume
                                    // Whole percents; a little either side of as recorded is as recorded.
                                    onMoved: v => page.set("editor", starting.modelData.key, Math.abs(v - 1) < 0.04 ? 1 : Math.round(v * 100) / 100)
                                    Accessible.name: "Initial volume: " + starting.modelData.label
                                }
                                Text {
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: Math.round(starting.volume * 100) + "%"
                                    color: Math.abs(starting.volume - 1) < 0.005 ? Theme.textFaint : Theme.text
                                    font.family: Theme.monoFamily
                                    font.pixelSize: Theme.body
                                    font.features: { "tnum": 1 }
                                }
                            }
                        }
                    }
                }
            }
            }

            Column {
            width: body.column
            spacing: 28

            SettingGroup {
                title: "Games"
                SettingRow {
                    label: "A game Zenith does not find"
                    note: Qt.platform.os === "windows"
                        ? "Games from Steam, Epic, GOG, Ubisoft, Battle.net, the EA app, Riot, Rockstar and Xbox are found by themselves. For one that is not, start it, then add it here: Zenith knows it by its program from then on."
                        : "Steam games are found by themselves. For one that is not, start it, then add it here: Zenith knows it by its program from then on."
                    Btn { implicitHeight: 32; icon: "plus"; text: "Add a game"; onClicked: addGame.ask() }
                }
                Repeater {
                    model: page.cfg.games.added
                    SettingRow {
                        required property var modelData
                        required property int index
                        label: modelData.name
                        note: modelData.program
                        IconButton {
                            icon: "trash"
                            tip: "Forget " + modelData.name
                            onClicked: page.set("games", "added", page.cfg.games.added.filter((g, i) => i !== index))
                        }
                    }
                }
            }

            SettingGroup {
                title: "When a clip is saved"
                SettingRow {
                    label: "Play a sound"
                    Toggle { on: page.cfg.feedback.sound; onToggled: on => page.set("feedback", "sound", on) }
                }
                SettingRow {
                    label: "Show a notification"
                    Toggle { on: page.cfg.feedback.notification; onToggled: on => page.set("feedback", "notification", on) }
                }
            }

            SettingGroup {
                title: "Colours"
                SettingRow {
                    label: "Highlight"
                    note: "Names, what is chosen and the part of a clip that is kept."
                    ColorChoice {
                        what: "Highlight colour"
                        standard: Theme.accentDefault
                        value: page.cfg.look.accent
                        onChosen: colour => page.set("look", "accent", colour)
                    }
                }
                SettingRow {
                    label: "Buttons"
                    note: "The main button, switches, sliders and where the keyboard is."
                    ColorChoice {
                        what: "Button colour"
                        standard: Theme.actionDefault
                        value: page.cfg.look.action
                        onChosen: colour => page.set("look", "action", colour)
                    }
                }
                SettingRow {
                    label: "Background"
                    note: "The window itself. Boxes and fields are a shade away from it."
                    ColorChoice {
                        what: "Background colour"
                        standard: Theme.bgDefault
                        others: ["#000000", "#121214", "#16110E", "#0A1024", "#0C1710", "#170F1F"]
                        names: ({ "#000000": "Black", "#121214": "Graphite", "#16110E": "Umber", "#0A1024": "Navy", "#0C1710": "Forest", "#170F1F": "Plum" })
                        value: page.cfg.look.background
                        onChosen: colour => page.set("look", "background", colour)
                    }
                }
                SettingRow {
                    label: "Lines"
                    note: "The lines between the parts of the window and around boxes."
                    ColorChoice {
                        what: "Line colour"
                        standard: Theme.lineDefault
                        others: ["#12181C", "#3F4F5A", "#5E6E78", "#3A3A40", "#3DE0F5", "#FF3DA5"]
                        names: ({ "#12181C": "Faint", "#3F4F5A": "Strong", "#5E6E78": "Bright", "#3A3A40": "Grey", "#3DE0F5": "Cyan", "#FF3DA5": "Magenta" })
                        value: page.cfg.look.line
                        onChosen: colour => page.set("look", "line", colour)
                    }
                }
            }

            SettingGroup {
                title: "Setup check"
                Checks { id: setupChecks }
                SettingRow {
                    label: "Check again"
                    note: "After installing something or changing a setting above."
                    Btn { implicitHeight: 32; icon: "refresh"; text: "Check"; onClicked: setupChecks.refresh() }
                }
                SettingRow {
                    label: "Something not working?"
                    note: "Copies a report to paste to whoever is helping: versions, this check, what the recorder is doing, and the last log lines of the recorder and of this window, from before this start too. They name the games played and clips saved. The clips themselves are not included."
                    Btn { implicitHeight: 32; icon: "copy"; text: "Copy report"; onClicked: Settings.copyReport() }
                }
            }

            // On Windows the recorder listens for the key itself.
            SettingGroup {
                visible: Qt.platform.os === "windows"
                title: "Shortcut"
                SettingRow {
                    label: "Clip key"
                    note: clipKey.problem !== "" ? clipKey.problem
                        : page.keyTrouble !== "" ? page.keyTrouble
                        : "Press this anywhere, in a game too, to save the last moments as a clip. Click it and press the key you want."
                    KeyPicker {
                        id: clipKey
                        value: page.cfg.capture.hotkey || "Alt+F9"
                        onPicked: combination => { if (combination !== page.cfg.capture.hotkey) { page.set("capture", "hotkey", combination); keyCheck.restart() } }
                    }
                }
                SettingRow {
                    label: "Start with Windows"
                    note: "The recorder starts when you sign in, so clips work without opening Zenith first."
                    Toggle {
                        id: atSignIn
                        on: Settings.startsAtSignIn()
                        onToggled: wanted => {
                            const wrong = Settings.setStartsAtSignIn(wanted)
                            if (wrong !== "") { page.say(wrong) }
                            on = Settings.startsAtSignIn()
                        }
                    }
                }
            }

            SettingGroup {
                visible: Qt.platform.os !== "windows"
                title: "Shortcut"
                // The desktop is asked for the key, where it can be asked
                // (it may show a question of its own, and may give another
                // key than the one wanted).
                SettingRow {
                    label: "Clip key"
                    note: linuxKey.problem !== "" ? linuxKey.problem
                        : page.keyHeld !== "" ? "Your desktop holds " + page.keyHeld + " for Zenith: press it anywhere, in a game too, to save the last moments as a clip."
                        : page.keyMissing !== "" ? "No key from your desktop: " + page.keyMissing + ". Use the command below instead."
                        : "Zenith asks your desktop for this key. Where the desktop cannot give one, use the command below instead."
                    KeyPicker {
                        id: linuxKey
                        value: page.cfg.capture.hotkey || "Alt+F9"
                        onPicked: combination => { if (combination !== page.cfg.capture.hotkey) { page.set("capture", "hotkey", combination); keyCheck.restart() } }
                    }
                }
                SettingRow {
                    label: "Clip shortcut"
                    note: "Or set it in your desktop's own keyboard settings: bind a key to the command below."
                    stacked: true
                    Rectangle {
                        width: parent.width
                        height: 40
                        radius: Theme.radius
                        color: Theme.bg
                        border.color: Theme.line
                        Text {
                            anchors.left: parent.left
                            anchors.leftMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            text: Settings.saveCommand()
                            elide: Text.ElideMiddle
                            width: parent.width - 56
                            color: Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.body
                        }
                        IconButton {
                            anchors.right: parent.right
                            anchors.rightMargin: 4
                            anchors.verticalCenter: parent.verticalCenter
                            icon: "copy"
                            tip: "Copy the command"
                            onClicked: { Settings.copyText(Settings.saveCommand()); page.say("Copied.") }
                        }
                    }
                }
            }

            SettingGroup {
                title: "Updates"
                SettingRow {
                    label: "Zenith " + page.up.current
                    note: page.up.rolledBack && page.up.state !== "available" ? "Version " + page.up.rolledBack + " did not start on this computer, so Zenith went back to this one."
                        : page.up.state === "checking" ? "Looking for a newer version…"
                        : page.up.state === "current" ? "This is the newest version."
                        : page.up.state === "available" ? "Version " + page.up.latest + " is out" + (page.up.megabytes > 0 ? ", " + Math.round(page.up.megabytes) + " MB." : ".")
                            + (page.up.here === "packaged" ? " This copy came from a package, so update it the same way." : page.up.here === "development" ? " This copy was built from the source and does not update itself." : "")
                        : page.up.state === "downloading" ? "Fetching version " + page.up.latest + "…  " + Math.round(page.up.progress * 100) + "%"
                        : page.up.state === "ready" ? "Version " + page.up.latest + " is in place. It runs from the next start."
                        : page.up.state === "failed" ? page.up.error
                        : page.up.here === "development" ? "Built from the source: it does not update itself."
                        : "Not looked yet."
                    Btn {
                        implicitHeight: 32
                        primary: page.up.state === "ready" || (page.up.state === "available" && page.up.here === "portable")
                        visible: page.up.state !== "downloading" && page.up.state !== "checking"
                        icon: page.up.state === "ready" || primary ? "" : "refresh"
                        text: page.up.state === "ready" ? "Restart now"
                            : page.up.state === "available" && page.up.here === "portable" ? "Update to " + page.up.latest
                            : "Check now"
                        onClicked: {
                            if (page.up.state === "ready") {
                                // Zenith only closes when the new version was started.
                                const why = Updates.restart()
                                if (why === "") { page.flush(); Qt.quit() } else { page.say(why) }
                            }
                            else if (page.up.state === "available" && page.up.here === "portable") { Updates.install() }
                            else { Updates.check(true) }
                        }
                    }
                }
                // The patch notes of the version running, to read when wanted.
                SettingRow {
                    visible: !!page.up.currentNotes
                    height: visible ? implicitHeight : 0
                    label: "What is new in " + page.up.current
                    note: "The patch notes of the version you are running."
                    Btn { implicitHeight: 32; text: page.showNotes ? "Hide" : "Show"; onClicked: page.showNotes = !page.showNotes }
                }
                Item {
                    visible: page.showNotes && !!page.up.currentNotes
                    width: parent.width
                    height: visible ? currentNotes.implicitHeight + 28 : 0
                    Text {
                        id: currentNotes
                        x: 18
                        y: 14
                        width: parent.width - 36
                        text: page.up.currentNotes || ""
                        textFormat: Text.MarkdownText
                        wrapMode: Text.WordWrap
                        color: Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.small
                        lineHeight: 1.25
                    }
                    Rectangle { anchors.bottom: parent.bottom; x: 18; width: parent.width - 36; height: 1; color: Theme.line; opacity: 0.5 }
                }
                // The patch notes of the version on offer, as they were written.
                SettingRow {
                    visible: page.up.notes !== "" && ["available", "downloading", "ready"].indexOf(page.up.state) >= 0
                    height: visible ? implicitHeight : 0
                    label: "What is new in " + page.up.latest
                    stacked: true
                    Text {
                        width: parent.width
                        text: page.up.notes
                        textFormat: Text.MarkdownText
                        wrapMode: Text.WordWrap
                        color: Theme.textDim
                        font.family: Theme.family
                        font.pixelSize: Theme.small
                        lineHeight: 1.25
                    }
                }
                SettingRow {
                    label: "Zenith on GitHub"
                    note: "Every version and its patch notes, and where to say what is broken or missing."
                    Btn { implicitHeight: 32; icon: "link"; text: "Open"; tip: page.github; onClicked: Qt.openUrlExternally(page.github) }
                }
                SettingRow {
                    label: "Look for new versions"
                    note: "Now and then Zenith asks GitHub whether there is a newer version. Nothing about you or your clips is sent."
                    Toggle { on: page.cfg.updates.check; onToggled: on => page.set("updates", "check", on) }
                }
            }
            }
        }
    }

    // Adding a game: one of the programs running now, under a name.
    Sheet {
        id: addGame
        title: "Add a game"
        panelWidth: 540
        // What was under it had the keyboard taken from it: the page has it again.
        onClosed: page.forceActiveFocus()
        // [{ program, title }]
        property var programs: []
        property string picked: ""
        readonly property var found: {
            const wanted = find.text.trim().toLowerCase()
            return programs.filter(p => wanted === "" || p.program.toLowerCase().indexOf(wanted) >= 0 || p.title.toLowerCase().indexOf(wanted) >= 0)
        }
        function ask() {
            programs = JSON.parse(Settings.runningPrograms())
            picked = ""
            find.text = ""
            gameName.value = ""
            gameName.text = ""
            show()
        }
        function choose(program) {
            picked = program.program
            const name = program.title !== "" ? program.title : program.program.replace(/\.exe$/i, "")
            gameName.value = name
            gameName.text = name
        }
        function add() {
            const name = gameName.text.trim()
            if (picked === "" || name === "") { return }
            const others = page.cfg.games.added.filter(g => g.program.toLowerCase() !== picked.toLowerCase())
            page.set("games", "added", others.concat([{ name: name, program: picked }]))
            close()
        }

        Text {
            width: parent.width
            text: "Start the game, then pick its program here. While that program runs, Zenith captures and files clips under the name below."
            wrapMode: Text.WordWrap
            color: Theme.textDim
            font.family: Theme.family
            font.pixelSize: Theme.small
            lineHeight: 1.25
        }
        SearchField { id: find; width: parent.width; placeholder: "Search what is running" }
        Flickable {
            width: parent.width
            height: Math.min(running.implicitHeight, 252)
            contentHeight: running.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            Column {
                id: running
                width: parent.width
                spacing: 2
                Repeater {
                    model: addGame.found
                    Rectangle {
                        id: program
                        required property var modelData
                        readonly property bool on: addGame.picked === modelData.program
                        width: parent.width
                        height: 40
                        radius: Theme.radius
                        color: on ? Theme.hover : over.containsMouse ? Theme.raised : "transparent"
                        border.color: on ? Theme.accent : "transparent"
                        activeFocusOnTab: true
                        Keys.onPressed: event => {
                            if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                                addGame.choose(modelData)
                                event.accepted = true
                            }
                        }
                        Accessible.role: Accessible.RadioButton
                        Accessible.name: modelData.title !== "" ? modelData.title + ", " + modelData.program : modelData.program
                        Accessible.checkable: true
                        Accessible.checked: on
                        Accessible.onPressAction: addGame.choose(modelData)
                        Text {
                            x: 12
                            width: parent.width - file.width - 36
                            anchors.verticalCenter: parent.verticalCenter
                            visible: program.modelData.title !== ""
                            text: program.modelData.title
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            color: Theme.text
                            font.family: Theme.family
                            font.pixelSize: Theme.body
                        }
                        Text {
                            id: file
                            x: program.modelData.title !== "" ? parent.width - width - 12 : 12
                            width: Math.min(implicitWidth, program.modelData.title !== "" ? 200 : parent.width - 24)
                            anchors.verticalCenter: parent.verticalCenter
                            text: program.modelData.program
                            textFormat: Text.PlainText
                            elide: Text.ElideMiddle
                            color: program.modelData.title !== "" ? Theme.textFaint : Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.small
                        }
                        MouseArea {
                            id: over
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: addGame.choose(program.modelData)
                        }
                        FocusRing {}
                    }
                }
            }
        }
        Text {
            visible: addGame.found.length === 0
            width: parent.width
            text: addGame.programs.length === 0 ? "No running programs were found." : "Nothing running goes by that name. Start the game, then open this again."
            wrapMode: Text.WordWrap
            color: Theme.textFaint
            font.family: Theme.family
            font.pixelSize: Theme.small
        }
        Row {
            width: parent.width
            spacing: 10
            Field {
                id: gameName
                width: parent.width - addButton.width - 10
                placeholder: "The name its clips go under"
                onCommitted: words => value = words
                onEntered: addGame.add()
            }
            Btn { id: addButton; primary: true; text: "Add"; enabled: addGame.picked !== "" && gameName.text.trim() !== ""; onClicked: addGame.add() }
        }
    }
}

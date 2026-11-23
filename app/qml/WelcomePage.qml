import QtQuick
import Zenith

// The first time the app is opened: a few steps, one screen each. What
// stands in the way of clipping (only when something does), the key that
// saves a clip, and what goes into one. Every choice is the settings' own,
// saved as it is made, and can be changed there later.
FocusScope {
    id: page
    signal closed()
    signal say(string message)

    // The settings, changed and saved as on the settings page.
    property var cfg: JSON.parse(Settings.load())
    function set(section, key, value) {
        const next = JSON.parse(JSON.stringify(cfg))
        next[section][key] = value
        cfg = next
        saver.restart()
    }
    function save() {
        const kept = JSON.parse(Settings.save(JSON.stringify(cfg)))
        if (kept.error) { page.say(kept.error) }
        if (kept.capture) { delete kept.error; cfg = kept }
    }
    function flush() { if (saver.running) { saver.stop(); save() } }
    Timer { id: saver; interval: 500; onTriggered: page.save() }
    Component.onDestruction: flush()

    // What the recorder says about itself: whether it runs, and on Linux
    // the key the desktop holds for it, or why it holds none.
    property var status: ({ running: false })
    Connections {
        target: Recorder
        function onStatusReady(status) { page.status = JSON.parse(status) }
    }
    // Asked often while the key is being settled: the desktop may first
    // show a question of its own.
    Timer { interval: 1500; running: page.step === "key"; repeat: true; triggeredOnStart: true; onTriggered: Recorder.poll() }

    // The games Zenith finds by itself, said where capture is chosen.
    readonly property var games: JSON.parse(Settings.gamesFound())
    // What a clip of this length comes to depends on the monitor, which is
    // known a moment after asking.
    property var machine: JSON.parse(Settings.machine())
    Connections {
        target: Settings
        function onMachineReady() { page.machine = JSON.parse(Settings.machine()) }
    }
    readonly property var estimate: { machine; return JSON.parse(Settings.estimate(JSON.stringify(cfg))) }

    // The steps there are: the first only while something is wrong. It is
    // decided once, when the check has answered, so steps do not come and
    // go under the person while they read.
    property bool fixFirst: false
    property bool decided: false
    readonly property var steps: (fixFirst ? ["ready"] : []).concat(["key", "record"])
    property int at: 0
    // `--welcome-at 2` opens on that step, for a look without a person.
    readonly property int lookAt: { const i = Qt.application.arguments.indexOf("--welcome-at"); return i >= 0 ? parseInt(Qt.application.arguments[i + 1]) || 0 : 0 }
    onDecidedChanged: if (decided && lookAt > 0) { at = Math.min(lookAt, steps.length - 1) }
    readonly property string step: !decided ? "checking" : steps[at]
    readonly property bool linux: Qt.platform.os !== "windows"

    // A check that never answers does not keep anyone at the door.
    Timer { interval: 6000; running: !page.decided; onTriggered: page.decided = true }

    function next() {
        if (at < steps.length - 1) { at += 1; flick.contentY = 0 }
        else { flush(); Settings.welcomeDone(); page.closed() }
    }
    function back() { if (at > 0) { at -= 1; flick.contentY = 0 } }

    function lengthWords(seconds) {
        if (seconds < 60) { return seconds + " seconds" }
        const m = Math.floor(seconds / 60)
        const s = seconds % 60
        return m + (m === 1 ? " minute" : " minutes") + (s ? " " + s + " s" : "")
    }
    // The key a clip is saved with, as the person will press it.
    readonly property string key: page.cfg.capture.hotkey || "Alt+F9"

    focus: true
    Keys.onEscapePressed: back()

    Rectangle { anchors.fill: parent; color: Theme.bg }
    Frit { anchors.top: parent.top; anchors.right: parent.right; width: 520; height: 240 }

    Flickable {
        id: flick
        anchors.fill: parent
        contentHeight: body.implicitHeight + 96
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: body
            y: 56
            width: Math.min(640, parent.width - 48)
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 28

            // Where this is: the logo, and how far along.
            Row {
                spacing: 16
                Logo { size: 40; z: 1; anchors.verticalCenter: parent.verticalCenter }
                Row {
                    visible: page.decided
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6
                    Repeater {
                        model: page.steps.length
                        Rectangle {
                            required property int index
                            width: index === page.at ? 28 : 8
                            height: 8
                            radius: 4
                            color: index <= page.at ? Theme.accent : Theme.line
                            Behavior on width { NumberAnimation { duration: Theme.smooth; easing.type: Easing.OutCubic } }
                        }
                    }
                }
                Text {
                    visible: page.decided
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Step " + (page.at + 1) + " of " + page.steps.length
                    color: Theme.textFaint
                    font.family: Theme.monoFamily
                    font.pixelSize: Theme.small
                    font.capitalization: Font.AllUppercase
                }
            }

            // The step's title and what it is for. The first step also says
            // what Zenith is.
            Column {
                width: parent.width
                spacing: 12
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: page.step === "checking" ? "Keep the moment that just happened"
                        : page.step === "ready" ? "First, something to fix"
                        : page.step === "key" ? "Pick your clip key"
                        : "What goes into a clip"
                    color: Theme.accentWords
                    font.family: Theme.titleFamily
                    font.pixelSize: 48
                    font.capitalization: Font.AllUppercase
                    lineHeight: 0.85
                }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    text: (page.at === 0 || page.step === "checking"
                            ? "While a game is running, Zenith remembers the last moments. Press your key and they are saved as a clip, ready to trim and share. "
                            : "")
                        + (page.step === "checking" ? "Checking this computer…"
                        : page.step === "ready" ? "Before anything can be clipped, this has to be put right."
                        : page.step === "key" ? "Press it anywhere, in a game too, and what just happened is saved."
                        : "Pick what you want; everything here can be changed later in the settings.")
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: 15
                    lineHeight: 1.35
                }
            }

            // Step: what stands in the way.
            SettingGroup {
                visible: page.step === "ready"
                width: parent.width
                // Checking from the start, out of sight until it has a
                // trouble to show.
                Checks {
                    id: checks
                    troubleOnly: true
                    onSay: message => page.say(message)
                    onReportChanged: {
                        if (!page.decided && report.worst !== "checking") {
                            page.fixFirst = report.worst === "problem"
                            page.decided = true
                        }
                    }
                }
                SettingRow {
                    visible: checks.report.worst !== "problem"
                    label: "All set"
                    note: "Nothing stands in the way any more."
                }
            }

            // Step: the key.
            SettingGroup {
                visible: page.step === "key"
                width: parent.width
                SettingRow {
                    label: "Clip key"
                    note: keyPicker.problem !== "" ? keyPicker.problem
                        : !page.linux ? (page.status.detail && /hotkey|key/i.test(page.status.detail) ? page.status.detail : "Click it and press the key you want.")
                        : !page.status.running ? "The recorder is not running, so the desktop cannot be asked for a key yet."
                        : page.status.hotkey ? "Your desktop holds " + page.status.hotkey + " for Zenith."
                        : page.status.hotkeyMissing ? "No key from your desktop: " + page.status.hotkeyMissing + ". Use the command below instead."
                        : "Zenith asks your desktop for this key. It may show a question of its own first."
                    KeyPicker {
                        id: keyPicker
                        value: page.key
                        onPicked: combination => {
                            problem = Settings.hotkeyProblem(combination)
                            if (problem === "" && combination !== page.cfg.capture.hotkey) {
                                page.set("capture", "hotkey", combination)
                                page.flush()
                            }
                        }
                    }
                }
                // Where the desktop gives no key (or cannot be asked), a key
                // bound in its own settings does the same.
                SettingRow {
                    visible: page.linux && (!!page.status.hotkeyMissing || !page.status.running)
                    label: "Or bind a key yourself"
                    note: "In your desktop's keyboard settings, add a custom shortcut that runs this."
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

            // Step: what is recorded, and when.
            SettingGroup {
                visible: page.step === "record"
                width: parent.width
                SettingRow {
                    label: "When to record"
                    note: page.cfg.capture.activation === "always"
                        ? "All the time, whatever is on screen."
                        : page.games.count > 0
                            ? "While a game runs. Zenith finds your games through " + page.games.from + ": " + page.games.count
                              + (page.games.count === 1 ? " game here" : " games here") + ", like " + page.games.names.join(", ")
                              + ". Any other game can be added in the settings."
                            : "While a game runs. Zenith finds games through " + page.games.from
                              + " and found none on this computer: add yours in the settings, or record always."
                    Segmented {
                        options: [{ key: "game", label: "During games" }, { key: "always", label: "Always" }]
                        value: page.cfg.capture.activation
                        onChosen: key => page.set("capture", "activation", key)
                    }
                }
                SettingRow {
                    label: "Clip length"
                    note: "How far back a clip reaches. Kept while capturing: about "
                        + Theme.size((page.estimate.megabytes || 0) * 1024 * 1024) + "."
                    Segmented {
                        readonly property var lengths: [30, 60, 120, 300]
                        options: lengths.map(s => ({ key: String(s), label: s < 60 ? s + " s" : (s / 60) + " min" }))
                            .concat(lengths.indexOf(page.cfg.capture.buffer_secs) < 0
                                ? [{ key: String(page.cfg.capture.buffer_secs), label: page.lengthWords(page.cfg.capture.buffer_secs) }] : [])
                        value: String(page.cfg.capture.buffer_secs)
                        onChosen: key => page.set("capture", "buffer_secs", parseInt(key))
                    }
                }
                SettingRow {
                    label: "Game sound"
                    note: "Everything the game plays, on its own track."
                    Toggle { on: page.cfg.audio.game; onToggled: on => page.set("audio", "game", on) }
                }
                SettingRow {
                    label: "Voice chat"
                    note: "Discord and the other voice apps, on a track of their own, so they can be turned down or muted afterwards."
                    Toggle { on: page.cfg.audio.chat; onToggled: on => page.set("audio", "chat", on) }
                }
                SettingRow {
                    label: "Your microphone"
                    note: "Your own voice, on its own track. Off if you would rather it was never in a clip."
                    Toggle { on: page.cfg.audio.microphone; onToggled: on => page.set("audio", "microphone", on) }
                }
            }

            Row {
                spacing: 10
                Btn {
                    visible: page.at > 0
                    implicitHeight: 42
                    text: "Back"
                    onClicked: page.back()
                }
                Btn {
                    primary: page.step !== "ready" || checks.report.worst !== "problem"
                    implicitHeight: 42
                    text: page.step === "checking" ? "Skip the check"
                        : page.step === "ready" && checks.report.worst === "problem" ? "Continue anyway"
                        : page.at === page.steps.length - 1 ? "Open my clips"
                        : "Continue"
                    onClicked: page.step === "checking" ? page.decided = true : page.next()
                }
                Btn {
                    visible: page.step === "ready"
                    implicitHeight: 42
                    icon: "refresh"
                    text: "Check again"
                    onClicked: checks.refresh()
                }
            }
        }
    }
}

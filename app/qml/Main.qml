import QtQuick
import QtQuick.Window
import Zenith

Window {
    id: window
    width: 1360
    height: 860
    minimumWidth: 900
    minimumHeight: 600
    visible: true
    title: "Zenith"
    color: Theme.bg

    // The window has no frame from the system: the bar at the top is the
    // app's own, with the buttons to minimise, fill the screen and close
    // in it, and the window is moved by that bar and sized by its edges.
    // (`--system-frame` keeps the system's.) On Windows the window still
    // is one of the system's own kind underneath (see `ownFrame` in
    // cpp/helpers.h), which is what snapping, its shadow and its rounded
    // corners hang on.
    readonly property bool ownFrame: (Qt.platform.os === "linux" || Qt.platform.os === "windows") && Qt.application.arguments.indexOf("--system-frame") < 0
    readonly property bool filled: visibility === Window.Maximized || visibility === Window.FullScreen
    flags: ownFrame ? Qt.Window | Qt.FramelessWindowHint : Qt.Window
    function toggleFilled() { if (filled) { showNormal() } else { showMaximized() } }
    // On Windows the button that fills the screen is Windows' to hear the
    // pointer and clicks on (which is what offers Windows 11's places to
    // snap to): where the button is, for Windows to know it by, and what
    // Windows says of the pointer on it. See `FrameFilter` in cpp/helpers.h.
    readonly property rect maxButton: ownFrame ? Qt.rect(frameButtons.x + 46, 0, 46, frameButtons.height) : Qt.rect(0, 0, 0, 0)
    property bool maxHovered: false
    property bool maxPressed: false

    // New versions: looked for a little after the window opens and a few
    // times a day after that, unless the settings say not to.
    property var updates: JSON.parse(Updates.status())
    Connections {
        target: Updates
        function onChanged() {
            const was = window.updates.state
            window.updates = JSON.parse(Updates.status())
            if (window.updates.state === "failed" && was === "downloading") { window.say("The update did not go in: " + window.updates.error) }
        }
    }
    // `--update`: look for a new version now, put it in place, say how it
    // went and quit. For updating from a terminal or a script.
    readonly property bool updateAndQuit: Qt.application.arguments.indexOf("--update") >= 0
    // Leaving waits a moment: work begun as the window opened (reading the
    // clip folder, asking the recorder) is not cut off half way.
    Timer { id: leaving; property int code: 0; interval: 600; onTriggered: Qt.exit(code) }
    function leave(words, code) { console.log("zenith-clips: " + words); leaving.code = code; leaving.start() }
    onUpdatesChanged: {
        if (!updateAndQuit || leaving.running) { return }
        if (updates.state === "available") {
            if (updates.here === "portable") { Updates.install() } else { leave("version " + updates.latest + " is out; this copy does not update itself", 0) }
        } else if (updates.state === "ready") { leave("version " + updates.latest + " is in place", 0) }
        else if (updates.state === "current") { leave("this is the newest version", 0) }
        else if (updates.state === "failed") { leave(updates.error, 1) }
    }
    Timer { interval: 20 * 1000; running: true; onTriggered: Updates.check(false) }
    Timer { interval: 6 * 60 * 60 * 1000; running: true; repeat: true; onTriggered: Updates.check(false) }
    // Ten seconds of running is a good start: a version on trial is kept.
    Timer { interval: 10 * 1000; running: true; onTriggered: Updates.startedWell() }

    // Start the new version in this one's place. Zenith only closes when
    // that was begun; when it could not be, it stays and says why.
    function restartToFinish() {
        const why = Updates.restart()
        if (why === "") { Qt.quit() } else { say(why) }
    }

    function say(message) {
        settleTrash()
        toast.text = message
        toast.shown = true
        toastTimer.interval = 3200
        toastTimer.restart()
    }
    // As `say`, with "Undo" beside the words for as long as they show:
    // clips that went to the trash can be brought back until then.
    function offerUndo(message) {
        toast.text = message
        toast.undo = true
        toast.shown = true
        toastTimer.interval = 7000
        toastTimer.restart()
    }
    function undo() {
        if (!toast.undo) { return }
        toast.undo = false
        library.undoTrash()
    }
    // The moment to take it back has passed: the files go to the trash.
    function settleTrash() {
        if (!toast.undo) { return }
        toast.undo = false
        library.finishTrash()
    }
    // Closing while an export runs is asked about first: it would stop
    // the export. Otherwise (or once that is answered) closing leaves
    // nothing half done: no clips half way to the trash, no setting changed
    // a moment ago and not kept yet, no last steps of a clip's sync unkept.
    // (What is still running then, an export that was to be stopped, is
    // seen to an end after the window has gone: see `main.rs`.)
    property bool closeAnyway: false
    onClosing: close => {
        if (Editor.exportBusy() && !closeAnyway) {
            close.accepted = false
            closeSheet.show()
            return
        }
        toast.undo = false
        Library.finishTrashNow()
        if (editor.item && editor.item.flush) { editor.item.flush() }
        Editor.settle()
        // Closed by its user is a start that went well, however short: a
        // version on trial is not taken back for being closed quickly.
        Updates.startedWell()
    }

    Rectangle { anchors.fill: parent; color: Theme.bg }

    Item {
        id: top
        // Everything under the question about closing waits for its answer.
        enabled: !closeSheet.open
        height: window.ownFrame ? 44 : 56
        anchors.left: parent.left
        anchors.right: parent.right

        // The bar is what the window is moved by.
        DragHandler {
            enabled: window.ownFrame
            target: null
            onActiveChanged: if (active) { window.startSystemMove() }
        }
        TapHandler {
            enabled: window.ownFrame
            gesturePolicy: TapHandler.DragThreshold
            onDoubleTapped: window.toggleFilled()
        }

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 24
            anchors.verticalCenter: parent.verticalCenter
            spacing: 10
            Logo { size: 24; anchors.verticalCenter: parent.verticalCenter }
            Text {
                text: "Zenith"
                color: Theme.text
                font.family: Theme.titleFamily
                font.pixelSize: 20
                font.capitalization: Font.AllUppercase
                font.letterSpacing: 0.4
                anchors.verticalCenter: parent.verticalCenter
            }
        }
        Row {
            // The library's own: the editor and the settings have theirs.
            visible: editor.item === null
            anchors.right: frameButtons.left
            anchors.rightMargin: window.ownFrame ? 10 : 20
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            // A new version: one press fetches it and puts it in place,
            // another starts it.
            Btn {
                anchors.verticalCenter: parent.verticalCenter
                // A version that did not start here is not pressed on anyone
                // again; the settings still offer it.
                visible: ["available", "downloading", "ready"].indexOf(window.updates.state) >= 0 && window.updates.rolledBack !== window.updates.latest
                implicitHeight: 30
                primary: true
                enabled: window.updates.state !== "downloading"
                text: window.updates.state === "ready" ? "Restart to finish"
                    : window.updates.state === "downloading" ? "Updating  " + Math.round(window.updates.progress * 100) + "%"
                    : "Update to " + window.updates.latest
                onClicked: {
                    if (window.updates.state === "ready") { window.restartToFinish() }
                    else if (window.updates.here === "portable") { Updates.install() }
                    else { editor.setSource("SettingsPage.qml", {}) }
                }
            }
            StatusPill { anchors.verticalCenter: parent.verticalCenter }
            IconButton { icon: "settings"; tip: "Settings"; onClicked: editor.setSource("SettingsPage.qml", {}) }
        }
        Row {
            id: frameButtons
            visible: window.ownFrame
            width: visible ? implicitWidth : 0
            anchors.right: parent.right
            height: parent.height
            Repeater {
                // Three, by count: what the middle one shows changes with
                // the window, and the buttons must outlive that. (Made
                // again from a new list, the one being clicked would go
                // while its click is still being heard.)
                model: 3
                Rectangle {
                    id: frameButton
                    required property int index
                    readonly property var modelData: [
                        { icon: "minus", name: "Minimise" },
                        { icon: window.filled ? "squares" : "square", name: window.filled ? "Restore" : "Maximise" },
                        { icon: "x", name: "Close" }
                    ][index]
                    function press() {
                        if (index === 0) { window.showMinimized() }
                        else if (index === 1) { window.toggleFilled() }
                        else { window.close() }
                    }
                    width: 46
                    height: frameButtons.height
                    // The pointer is on it: as the app sees it, or, for
                    // the button that fills the screen, as Windows says.
                    readonly property bool over: frameArea.containsMouse || (index === 1 && window.maxHovered)
                    readonly property bool down: frameArea.pressed || (index === 1 && window.maxPressed)
                    color: !over ? "transparent" : index === 2 ? Theme.record : down ? Theme.hover : Theme.raised
                    Behavior on color { ColorAnimation { duration: Theme.quick } }
                    Accessible.role: Accessible.Button
                    Accessible.name: modelData.name
                    Accessible.onPressAction: press()
                    Icon {
                        anchors.centerIn: parent
                        name: frameButton.modelData.icon
                        size: 15
                        color: frameButton.over ? (frameButton.index === 2 ? "#FFFFFF" : Theme.text) : Theme.textDim
                    }
                    MouseArea { id: frameArea; anchors.fill: parent; hoverEnabled: true; onClicked: frameButton.press() }
                }
            }
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line; opacity: 0.6 }
    }

    LibraryPage {
        id: library
        enabled: !closeSheet.open
        visible: editor.item === null
        anchors.top: top.bottom
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        onSay: message => window.say(message)
        onOfferUndo: message => window.offerUndo(message)
        onUndoAsked: window.undo()
        onOpenClip: clip => editor.setSource("EditorPage.qml", { entry: clip })
    }

    // The editor and the settings take the whole window, over the library.
    // (Under the window's own bar, where it has one.)
    Rectangle { anchors.fill: editor; color: Theme.bg; visible: editor.item !== null }
    Loader {
        id: editor
        enabled: !closeSheet.open
        anchors.fill: parent
        anchors.topMargin: window.ownFrame ? top.height : 0
        focus: true
        onLoaded: item.forceActiveFocus()
    }
    Connections {
        target: editor.item
        // Not every page has every one of these to say.
        ignoreUnknownSignals: true
        function onClosed() { editor.source = ""; library.refresh() }
        // From one clip to another: once the page that asked is done with
        // asking, it goes and the other clip's comes.
        function onOpenClip(clip) { Qt.callLater(() => { editor.source = ""; editor.setSource("EditorPage.qml", { entry: clip }) }) }
        function onSay(message) { window.say(message) }
    }

    // `--edit` opens the newest clip in the editor at once; `--size
    // 2560x1400` opens the window that big, for a look at a large screen.
    Component.onCompleted: {
        const size = Qt.application.arguments.indexOf("--size")
        if (size >= 0) {
            const wanted = (Qt.application.arguments[size + 1] || "").split("x").map(n => parseInt(n))
            if (wanted[0] > 0 && wanted[1] > 0) { width = wanted[0]; height = wanted[1] }
        }
        if (updateAndQuit) { Updates.check(true) }
        if (Settings.firstRun() || Qt.application.arguments.indexOf("--welcome") >= 0) {
            editor.setSource("WelcomePage.qml", {})
        }
        if (Qt.application.arguments.indexOf("--settings") >= 0) {
            editor.setSource("SettingsPage.qml", {})
        }
        const open = Qt.application.arguments.indexOf("--open")
        if (open >= 0) {
            Library.rescan()
            const wanted = Qt.application.arguments[open + 1]
            const found = JSON.parse(Library.clips("{}")).find(c => c.path === wanted)
            if (found) { editor.setSource("EditorPage.qml", { entry: found }) }
        }
        if (Qt.application.arguments.indexOf("--edit") >= 0) {
            // A library not read yet (a test's fresh one) has its clips first.
            Library.rescan()
            const clips = JSON.parse(Library.clips("{}"))
            const pick = clips.find(c => c.game !== "Desktop") || clips[0]
            if (pick) { editor.setSource("EditorPage.qml", { entry: pick }) }
        }
    }

    Rectangle {
        id: toast
        property bool shown: false
        property alias text: toastText.text
        // "Undo" is offered beside the words.
        property bool undo: false
        readonly property int extra: undo ? undoButton.width + 12 : 0
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        // Over the library's selection bar, when that is there.
        anchors.bottomMargin: (shown ? 28 : 8) + (library.visible ? library.lift : 0)
        opacity: shown ? 1 : 0
        visible: opacity > 0
        // As wide as its words, up to the window: longer ones wrap.
        width: Math.min(parent.width - 48, toastText.implicitWidth + 32 + extra)
        height: Math.max(40, toastText.implicitHeight + 20)
        radius: Theme.radius
        // A box of the window's own, a step lighter than what it lies
        // on, with the highlight down its side to catch the eye.
        color: Theme.hover
        border.color: Theme.line
        clip: true
        Rectangle { width: 3; height: parent.height; color: Theme.accent }
        Behavior on opacity { NumberAnimation { duration: Theme.smooth } }
        Behavior on anchors.bottomMargin { NumberAnimation { duration: Theme.smooth; easing.type: Easing.OutCubic } }
        Text {
            id: toastText
            textFormat: Text.PlainText
            anchors.verticalCenter: parent.verticalCenter
            x: 16
            width: parent.width - 32 - toast.extra
            horizontalAlignment: toast.undo ? Text.AlignLeft : Text.AlignHCenter
            wrapMode: Text.Wrap
            color: Theme.text
            font.family: Theme.family
            font.pixelSize: Theme.body
            font.weight: Font.Medium
        }
        Rectangle {
            id: undoButton
            visible: toast.undo
            anchors.right: parent.right
            anchors.rightMargin: 6
            anchors.verticalCenter: parent.verticalCenter
            width: undoWords.implicitWidth + 24
            height: 28
            radius: 2
            color: undoArea.containsMouse ? Theme.actionHover : Theme.action
            activeFocusOnTab: true
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { window.undo(); event.accepted = true }
            }
            Accessible.role: Accessible.Button
            Accessible.name: "Undo"
            Accessible.onPressAction: window.undo()
            Text {
                id: undoWords
                anchors.centerIn: parent
                text: "Undo"
                color: Theme.actionInk
                font.family: Theme.family
                font.pixelSize: Theme.body
                font.weight: Font.DemiBold
            }
            MouseArea {
                id: undoArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: window.undo()
            }
            FocusRing {}
        }
        Timer { id: toastTimer; interval: 3200; onTriggered: { window.settleTrash(); toast.shown = false } }
    }

    // Closing with an export running: it is the user's to say that the
    // export is to be stopped.
    Sheet {
        id: closeSheet
        title: "Stop the export and close?"
        onClosed: (editor.item || library).forceActiveFocus()
        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "An export is still running. Closing Zenith now stops it, and nothing of it is kept."
            color: Theme.textDim
            font.family: Theme.family
            font.pixelSize: Theme.body
            lineHeight: 1.3
        }
        Row {
            anchors.right: parent.right
            spacing: 10
            Btn { text: "Keep exporting"; onClicked: closeSheet.close() }
            Btn {
                primary: true
                text: "Stop and close"
                onClicked: { closeSheet.open = false; window.closeAnyway = true; Editor.exportCancel(); window.close() }
            }
        }
    }

    // The window's own frame: a line around it, and its edges and corners
    // to size it by. The line stays when the window fills the screen; the
    // edges are not for sizing then.
    Item {
        anchors.fill: parent
        visible: window.ownFrame
        z: 100
        Rectangle { anchors.fill: parent; color: "transparent"; border.width: 1; border.color: Theme.line }
        Repeater {
            model: [
                { edges: Qt.TopEdge, cursor: Qt.SizeVerCursor }, { edges: Qt.BottomEdge, cursor: Qt.SizeVerCursor },
                { edges: Qt.LeftEdge, cursor: Qt.SizeHorCursor }, { edges: Qt.RightEdge, cursor: Qt.SizeHorCursor },
                { edges: Qt.TopEdge | Qt.LeftEdge, cursor: Qt.SizeFDiagCursor }, { edges: Qt.BottomEdge | Qt.RightEdge, cursor: Qt.SizeFDiagCursor },
                { edges: Qt.TopEdge | Qt.RightEdge, cursor: Qt.SizeBDiagCursor }, { edges: Qt.BottomEdge | Qt.LeftEdge, cursor: Qt.SizeBDiagCursor }
            ]
            MouseArea {
                required property var modelData
                readonly property bool corner: (modelData.edges & (Qt.TopEdge | Qt.BottomEdge)) && (modelData.edges & (Qt.LeftEdge | Qt.RightEdge))
                readonly property int reach: corner ? 10 : 4
                x: modelData.edges & Qt.RightEdge ? parent.width - width : 0
                y: modelData.edges & Qt.BottomEdge ? parent.height - height : 0
                width: modelData.edges & (Qt.LeftEdge | Qt.RightEdge) ? reach : parent.width
                height: modelData.edges & (Qt.TopEdge | Qt.BottomEdge) ? reach : parent.height
                enabled: !window.filled
                cursorShape: modelData.cursor
                onPressed: window.startSystemResize(modelData.edges)
            }
        }
    }

    // The test modes below (`--selftest`, `--scrolltest`, `--librarytest`,
    // `--resizetest`) never work on the user's own settings, library or
    // clips: `main.rs` gives them a folder of their own before anything is
    // read, with copies of the video named by `--test-clip` (or after
    // `--librarytest`) as its clips.
    //
    // `--edit --selftest`: edit the newest clip the way a person would (two
    // cuts, the middle removed, a track muted), export it for Discord, say
    // how that went on the terminal, and quit. Leaves the clip as it was.
    // `--settings --selftest`: change three settings and keep them.
    Timer {
        interval: 1500
        running: Qt.application.arguments.indexOf("--selftest") >= 0
        onTriggered: {
            const page = editor.item
            if (!page) { console.log("selftest: no clip to edit"); Qt.quit(); return }
            if (page.cfg) {
                // The settings page: change three things and keep them.
                page.set("capture", "fps", 30)
                page.set("capture", "buffer_secs", 90)
                page.set("capture", "resolution", "1920x1080")
                page.save()
                console.log("selftest: settings", JSON.stringify(page.cfg.capture), JSON.stringify(page.estimate))
                Qt.quit()
                return
            }
            const d = page.edit.duration
            page.apply(Editor.split(d * 0.3))
            page.apply(Editor.split(d * 0.6))
            page.apply(Editor.remove(1))
            page.apply(Editor.setMuted(page.edit.gains.length - 1, true))
            console.log("selftest: kept", page.edit.kept.toFixed(2), "of", d.toFixed(2), "in", page.edit.segments.length, "parts; undo", page.edit.canUndo)
            console.log("selftest: plan", Editor.plan(20))
            Editor.exportStart(20)
        }
    }
    Connections {
        target: Editor
        enabled: Qt.application.arguments.indexOf("--selftest") >= 0
        function onExportFinished(clip, path, bytes, error) {
            console.log("selftest: exported", path, bytes, "bytes", error === "" ? "ok" : error)
            // Every step taken back, one at a time: the clip as it was.
            const page = editor.item
            for (let steps = 0; page.edit.canUndo && steps < 50; steps++) { page.apply(Editor.undo()) }
            console.log("selftest: undone; the clip is", page.edit.changed ? "still changed" : "as it was")
            Qt.quit()
        }
    }

    // `--scrolltest`: for checking a large library without a person. Runs
    // the library from top to bottom in four seconds, says on the terminal
    // how long it took to open, how many cards were made and how the
    // frames came, and quits.
    readonly property real born: Date.now()
    property real firstFrame: 0
    onFrameSwapped: {
        const now = Date.now()
        if (firstFrame === 0) { firstFrame = now - born }
        if (scrollRun.running) {
            scrollTest.frames += 1
            if (scrollTest.last > 0) { scrollTest.worst = Math.max(scrollTest.worst, now - scrollTest.last) }
            scrollTest.last = now
        }
    }
    Timer {
        id: scrollTest
        property int frames: 0
        property real worst: 0
        property real last: 0
        property int cardsAtOpen: 0
        interval: 3000
        running: Qt.application.arguments.indexOf("--scrolltest") >= 0
        onTriggered: { cardsAtOpen = library.cardsMade; scrollRun.start() }
    }
    NumberAnimation {
        id: scrollRun
        target: library.scroller
        property: "contentY"
        from: 0
        to: Math.max(0, library.scroller.contentHeight - library.scroller.height)
        duration: 4000
        onFinished: {
            console.log("scrolltest:", library.clips.length, "clips; first frame after", window.firstFrame, "ms; listed in", library.listMs, "ms, laid out in", library.drawMs, "ms;",
                scrollTest.cardsAtOpen, "cards made at open,", library.cardsMade - scrollTest.cardsAtOpen, "more while scrolling;",
                scrollTest.frames, "frames in 4 s, the longest wait between two", scrollTest.worst, "ms")
            Qt.quit()
        }
    }

    // `--librarytest`: see LibraryTest.qml.
    Loader {
        active: Qt.application.arguments.indexOf("--librarytest") >= 0
        source: "LibraryTest.qml"
        onLoaded: { item.page = library; item.window = window }
    }

    // `--resizetest`: the window is sized back and forth quickly for a few
    // seconds, as by a hand at its edge, and the program quits. The log
    // then says how drawing went meanwhile (see `watchSizing`).
    SequentialAnimation {
        running: Qt.application.arguments.indexOf("--resizetest") >= 0
        PauseAnimation { duration: 3000 }
        SequentialAnimation {
            loops: 3
            ParallelAnimation {
                NumberAnimation { target: window; property: "width"; to: 2300; duration: 500; easing.type: Easing.InOutQuad }
                NumberAnimation { target: window; property: "height"; to: 1340; duration: 500; easing.type: Easing.InOutQuad }
            }
            ParallelAnimation {
                NumberAnimation { target: window; property: "width"; to: 1100; duration: 500; easing.type: Easing.InOutQuad }
                NumberAnimation { target: window; property: "height"; to: 700; duration: 500; easing.type: Easing.InOutQuad }
            }
        }
        PauseAnimation { duration: 1500 }
        ScriptAction { script: Qt.quit() }
    }

    // For checking a screen without a person looking: `--shot file.png`
    // saves a picture of the window there and quits.
    Timer {
        readonly property int at: Qt.application.arguments.indexOf("--shot")
        interval: 2500
        running: at >= 0
        onTriggered: window.contentItem.grabToImage(function (result) {
            result.saveToFile(Qt.application.arguments[at + 1])
            Qt.quit()
        })
    }
}

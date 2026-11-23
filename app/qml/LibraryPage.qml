import QtQuick
import Zenith

// The clip library: where to look on the left, the clips on the right,
// grouped by game or by the day they were saved. Clips are picked one or
// many (the box on a card, Ctrl or Shift and a click, or the keyboard) and
// what is done next is done to all of them.
Item {
    id: page
    signal openClip(var clip)
    signal say(string message)
    // Clips went to the trash and can still be brought back: `undoTrash`
    // does that, `finishTrash` lets them go.
    signal offerUndo(string message)
    // Ctrl+Z was pressed.
    signal undoAsked()

    // "", "★" for favourites, or a game's name.
    property string place: ""
    // Only clips with this tag, in every game.
    property string tag: ""
    // "day" or "game"
    property string grouping: "day"
    property string sort: "newest"
    property var shelves: []
    // Each game's cover, by the game's name, for the cards.
    readonly property var covers: { const found = {}; for (const s of shelves) { if (s.cover) { found[s.game] = s.cover } } return found }
    property var tags: []
    // The clips listed, in the order they are shown.
    property var clips: []
    // What the recorder says, for an empty library to tell what is next:
    // the key, how far back a clip reaches, whether it runs at all.
    property var recorder: ({ running: true })
    Connections {
        target: Recorder
        function onStatusReady(status) { page.recorder = JSON.parse(status) }
    }
    // The games Zenith finds by itself, asked once, when there are no clips.
    property var games: null
    // The first clip is shown where to go next, once.
    property bool firstClipHint: !Settings.hintSeen("first-clip")
    // The rows of the grid: up to `columns` clips each, the first of a
    // group under its heading.
    property var rows: []
    property int total: 0
    // The shelves as they were last read, to tell when the library changed.
    property string shelvesText: ""

    // The picked clips, by id.
    property var picked: ({})
    property int pickedCount: 0
    // The card the keyboard is on, and the one a Shift range starts from.
    property int current: -1
    property int anchor: -1
    // The keyboard is being used on the cards, so its place is shown.
    property bool byKeys: false
    // More clips than this are not sent to the trash without asking.
    readonly property int manyClips: 20
    // The room the selection bar takes at the foot, for the window's toast.
    readonly property int lift: pickedCount > 0 ? 64 : 0

    // For `--scrolltest`: how long the last listing took, and how many
    // cards have been made since the window opened.
    property alias scroller: grid
    // For `--librarytest`: what is typed in the search, and a way out of
    // whichever sheet is open.
    property alias searchText: search.text
    function closeSheets() { detailsSheet.close(); moveSheet.close(); sure.close() }
    property int cardsMade: 0
    property real listMs: 0
    property real drawMs: 0

    readonly property var sorts: [
        { name: "newest", label: "Newest first" },
        { name: "oldest", label: "Oldest first" },
        { name: "longest", label: "Longest first" },
        { name: "shortest", label: "Shortest first" },
        { name: "largest", label: "Largest first" },
        { name: "smallest", label: "Smallest first" }
    ]

    // Read the library again. `listOnly` leaves the games' shelves as they
    // are, for when only what is listed changed (a search, another place).
    // What an empty library says: the key, how far back a clip reaches,
    // where games come from, and that videos can be brought in.
    function firstWords() {
        if (page.games === null) { page.games = JSON.parse(Settings.gamesFound()) }
        const key = page.recorder.hotkey || ""
        const secs = page.recorder.bufferSecs || 0
        const back = secs === 60 ? "minute" : secs > 0 && secs % 60 === 0 ? secs / 60 + " minutes" : secs > 0 ? secs + " seconds" : "moments"
        const lines = []
        if (!page.recorder.running) {
            lines.push("The recorder is not running, so nothing is being kept.")
        } else {
            lines.push("Press " + (key ? key : "your clip key") + " while a game is running\nand what happened in the last " + back + " lands here.")
        }
        if (page.games.count > 0) {
            lines.push("Zenith knows " + page.games.count + " of your games through " + (Qt.platform.os === "windows" ? "your launchers" : "Steam") + ".\nAny other can be added in the settings.")
        } else {
            lines.push("Zenith found no games here by itself:\nadd yours in the settings, or record always.")
        }
        lines.push("Videos made elsewhere can be dropped onto this window.")
        return lines.join("\n\n")
    }

    function refresh(listOnly) {
        if (!listOnly) {
            shelvesText = Library.shelves()
            shelves = JSON.parse(shelvesText)
            tags = JSON.parse(Library.tags())
            total = shelves.reduce((n, s) => n + s.clips, 0)
            if (place !== "" && place !== "★" && !shelves.some(s => s.game === place)) {
                place = ""
            }
            if (tag !== "" && !tags.some(t => t.tag === tag)) {
                tag = ""
            }
        }
        const filter = { search: search.text, sort: sort, tag: tag }
        if (place === "★") {
            filter.favorites = true
        } else if (place !== "") {
            filter.game = place
        }
        const before = Date.now()
        const listed = JSON.parse(Library.clips(JSON.stringify(filter)))
        listMs = Date.now() - before
        group(listed)
        keepPicked()
        drawMs = Date.now() - before - listMs
    }

    // Put the listed clips under their headings, in the order the grid
    // shows them. By date, a heading is a day; in another order the days
    // would come apart, so there is one heading for all.
    function group(listed) {
        const byDate = sort === "newest" || sort === "oldest"
        const byKey = new Map()
        for (const clip of listed) {
            const key = grouping === "game" ? clip.game : byDate ? Theme.day(clip.savedAt) : sorts.find(s => s.name === sort).label
            if (!byKey.has(key)) {
                byKey.set(key, [])
            }
            byKey.get(key).push(clip)
        }
        groups = Array.from(byKey, ([name, list]) => ({ name: name, clips: list }))
        clips = groups.reduce((all, g) => all.concat(g.clips), [])
        layout()
    }
    property var groups: []

    // Cut the groups into the grid's rows. Again when the window's width
    // changes how many cards fit side by side. How tall the cards are is
    // not in the rows (it changes with every step of the window being
    // sized, and the rows would be cut again at each): a row says only
    // what it adds to a card's height, in `h`.
    function layout() {
        const out = []
        let first = 0
        for (const g of groups) {
            for (let i = 0; i < g.clips.length; i += grid.columns) {
                const head = i === 0 ? { name: g.name, count: g.clips.length, top: out.length === 0 ? 0 : 8 } : null
                const some = g.clips.slice(i, i + grid.columns)
                out.push({ head: head, headH: head ? head.top + 48 : 0, clips: some, first: first, h: head ? head.top + 48 : 0 })
                first += some.length
            }
        }
        rows = out
    }

    // A clip that is not listed any more is not picked any more.
    function keepPicked() {
        const still = {}
        let n = 0
        for (const clip of clips) {
            if (picked[clip.id]) { still[clip.id] = true; n += 1 }
        }
        picked = still
        pickedCount = n
        current = Math.min(current, clips.length - 1)
        anchor = Math.min(anchor, clips.length - 1)
    }

    function setPicked(ids) {
        const now = {}
        for (const id of ids) { now[id] = true }
        picked = now
        pickedCount = ids.length
    }
    function clearPicked() { setPicked([]) }
    function togglePicked(index) {
        const id = clips[index].id
        const ids = Object.keys(picked).map(Number)
        const at = ids.indexOf(id)
        if (at >= 0) { ids.splice(at, 1) } else { ids.push(id) }
        setPicked(ids)
        anchor = index
        current = index
    }
    // Every clip from where the range began to `index`.
    function pickRange(index) {
        const from = Math.max(0, Math.min(anchor < 0 ? index : anchor, index))
        const to = Math.max(anchor, index)
        setPicked(clips.slice(from, to + 1).map(c => c.id))
        current = index
    }
    readonly property var pickedClips: pickedCount === 0 ? [] : clips.filter(c => picked[c.id])

    // What an action on `clip` is for: every picked clip when it is one of
    // them, else it alone.
    function targets(clip) {
        return clip && !picked[clip.id] ? [clip] : pickedCount > 0 ? pickedClips : clip ? [clip] : []
    }
    function idsOf(list) { return JSON.stringify(list.map(c => c.id)) }
    function clipsWord(n) { return n === 1 ? "1 clip" : n + " clips" }

    // A card was clicked: with Ctrl, Shift, or while clips are being
    // picked, it is picked; otherwise it opens.
    function press(index, modifiers) {
        byKeys = false
        grid.forceActiveFocus()
        if (modifiers & Qt.ShiftModifier) { pickRange(index) }
        else if ((modifiers & Qt.ControlModifier) || pickedCount > 0) { togglePicked(index) }
        else { current = index; openClip(clips[index]) }
    }

    function favorite(list) {
        if (list.length === 0) { return }
        const on = !list.every(c => c.favorite)
        Library.setFavorites(idsOf(list), on)
        refresh(true)
        if (list.length > 1) { say(clipsWord(list.length) + (on ? " added to favourites." : " removed from favourites.")) }
    }
    function copy(list) {
        if (list.length === 0) { return }
        Library.copyToClipboard(list.map(c => c.path).join("\n"))
        say(list.length === 1 ? "Copied. Paste it into a chat to upload it." : clipsWord(list.length) + " copied. Paste them into a chat to upload them.")
    }
    // A few clips go at once, since that can be taken back; many are
    // asked about first.
    function trash(list) {
        if (list.length === 0) { return }
        if (list.length > manyClips) { sure.list = list; sure.show(); return }
        trashNow(list)
    }
    function trashNow(list) {
        const n = Library.trash(idsOf(list))
        if (n === 0) { say("The clips could not be moved to the trash."); return }
        refresh()
        offerUndo(n === 1 ? "Moved to the trash." : n + " clips moved to the trash.")
    }
    function undoTrash() {
        const n = Library.undoTrash()
        refresh()
        say(n === 0 ? "There was nothing to bring back." : n === 1 ? "The clip is back." : n + " clips are back.")
    }
    function finishTrash() { Library.finishTrash() }
    function details(list) { if (list.length > 0) { detailsSheet.edit(list) } }
    function move(list) { if (list.length > 0) { moveSheet.list = list; moveSheet.ask(list.length, shelves) } }

    // The row a clip is in, and the clip a row up or down from another:
    // the one under it, or the row's last when the row is shorter.
    function rowOf(index) {
        let low = 0
        let high = rows.length - 1
        while (low < high) {
            const middle = (low + high + 1) >> 1
            if (rows[middle].first <= index) { low = middle } else { high = middle - 1 }
        }
        return low
    }
    function vertical(index, by) {
        const from = rowOf(index)
        const to = Math.max(0, Math.min(rows.length - 1, from + by))
        if (to === from) { return index }
        return rows[to].first + Math.min(index - rows[from].first, rows[to].clips.length - 1)
    }
    // Move the keyboard's place; with Shift the clips on the way are picked.
    function go(index, extend) {
        if (clips.length === 0) { return }
        index = Math.max(0, Math.min(clips.length - 1, index))
        byKeys = true
        if (extend) {
            if (anchor < 0) { anchor = Math.max(0, current) }
            pickRange(index)
        } else {
            current = index
            anchor = index
        }
        grid.reveal(rowOf(index))
    }

    // What is indexed shows at once; the folder is read on another thread.
    Component.onCompleted: { refresh(); Library.finishOldTrash(); Library.rescanStart() }
    function relist() { grid.contentY = 0; current = -1; anchor = -1; refresh(true) }
    onPlaceChanged: relist()
    onTagChanged: relist()
    onGroupingChanged: relist()
    onSortChanged: relist()

    // New clips appear by themselves while the window is open. The grid
    // is only made again when something did change.
    Timer {
        interval: 4000
        running: page.visible
        repeat: true
        onTriggered: Library.rescanStart()
    }
    Connections {
        target: Library
        function onRescanFinished(changed) {
            // The recorder adds its clips to the index itself, so the
            // shelves are compared too.
            if (changed > 0 || Library.shelves() !== page.shelvesText) { page.refresh() }
        }
        // Only said when some files could not be moved to the trash.
        function onTrashed(kept, problem) {
            page.refresh()
            page.say((kept === 1 ? "A clip is back: it" : kept + " clips are back: they") + " could not be moved to the trash. " + problem)
        }
        function onImported(added, problem) {
            page.refresh()
            page.say(problem !== "" ? (added > 0 ? page.clipsWord(added) + " added. " : "") + "Not added: " + problem
                : added === 0 ? "There was no video in what was dropped."
                : page.clipsWord(added) + " added to the library.")
        }
    }
    // Typing a word is one search, when the typing pauses.
    Timer { id: searching; interval: 150; onTriggered: page.relist() }

    // Everything under a sheet waits for it: no key or Tab reaches it.
    Item {
        anchors.fill: parent
        enabled: !detailsSheet.open && !moveSheet.open && !sure.open

        Rectangle {
            id: side
            width: 248
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            color: Theme.bg

            // Dots gathering in the bottom corner, under the list.
            Frit { anchors.bottom: parent.bottom; anchors.left: parent.left; width: parent.width; height: 230; rotation: 180 }

            Flickable {
                anchors.fill: parent
                anchors.margins: 12
                contentHeight: places.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                Column {
                    id: places
                    width: parent.width
                    spacing: 2
                    component Heading: Text {
                        leftPadding: 12
                        topPadding: 16
                        bottomPadding: 6
                        color: Theme.textFaint
                        font.family: Theme.monoFamily
                        font.pixelSize: 11
                        font.letterSpacing: 1.6
                    }
                    SideItem {
                        label: "All clips"
                        icon: "film"
                        count: page.total
                        selected: page.place === "" && page.tag === ""
                        onClicked: { page.tag = ""; page.place = "" }
                    }
                    SideItem {
                        label: "Favourites"
                        icon: "star"
                        selected: page.place === "★"
                        onClicked: { page.tag = ""; page.place = "★" }
                    }
                    Heading { visible: page.shelves.length > 0; text: "GAMES" }
                    Repeater {
                        model: page.shelves
                        SideItem {
                            required property var modelData
                            label: modelData.game
                            icon: "gamepad"
                            cover: modelData.cover || ""
                            count: modelData.clips
                            selected: page.place === modelData.game
                            onClicked: { page.tag = ""; page.place = modelData.game }
                        }
                    }
                    Heading { visible: page.tags.length > 0; text: "TAGS" }
                    Repeater {
                        model: page.tags
                        SideItem {
                            required property var modelData
                            label: modelData.tag
                            icon: "tag"
                            count: modelData.clips
                            selected: page.tag === modelData.tag
                            onClicked: { page.place = ""; page.tag = modelData.tag }
                        }
                    }
                }
            }
            Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.line; opacity: 0.6 }
        }

        Item {
            id: main
            anchors.left: side.right
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom

            Item {
                id: bar
                height: 92
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 28
                anchors.rightMargin: 28
                // Over the grid: what scrolls up goes under the bar.
                z: 1
                Rectangle { anchors.fill: parent; anchors.leftMargin: -28; anchors.rightMargin: -28; color: Theme.bg }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.right: barTools.left
                    anchors.rightMargin: 12
                    elide: Text.ElideRight
                    text: page.tag !== "" ? "#" + page.tag : page.place === "" ? "All clips" : page.place === "★" ? "Favourites" : page.place
                    color: Theme.accentWords
                    font.family: Theme.titleFamily
                    font.pixelSize: 46
                    font.capitalization: Font.AllUppercase
                }
                Row {
                    id: barTools
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 10
                    SearchField {
                        id: search
                        // Narrower in a small window, so the title keeps some room.
                        implicitWidth: Math.max(150, Math.min(260, bar.width - 444))
                        placeholder: "Search clips, tags, notes"
                        onTextChanged: searching.restart()
                    }
                    Segmented {
                        options: [
                            { key: "day", label: "By day", icon: "clock" },
                            { key: "game", label: "By game", icon: "gamepad" }
                        ]
                        value: page.grouping
                        onChosen: key => page.grouping = key
                    }
                    SortButton {
                        anchors.verticalCenter: parent.verticalCenter
                        options: page.sorts
                        value: page.sort
                        onChosen: name => page.sort = name
                    }
                    IconButton {
                        anchors.verticalCenter: parent.verticalCenter
                        icon: "folder"
                        tip: "Open the clip folder"
                        onClicked: Qt.openUrlExternally(Library.fileUrl(Library.folder()))
                    }
                }
            }

            // The first clip there is: what can be done with it, once.
            Rectangle {
                id: hint
                visible: page.firstClipHint && page.total > 0 && search.text === "" && page.place === "" && page.tag === ""
                anchors.top: bar.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: 28
                anchors.rightMargin: 28
                height: visible ? hintWords.implicitHeight + 28 : 0
                radius: Theme.radiusLarge
                color: Theme.surface
                border.color: Theme.accent
                Text {
                    id: hintWords
                    anchors.left: parent.left
                    anchors.right: hintDone.left
                    anchors.leftMargin: 16
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.WordWrap
                    text: "Your first clip is here. Open it to trim it, cut parts out, turn the voices down, and export it at a size Discord takes."
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.body
                    lineHeight: 1.3
                }
                Btn {
                    id: hintDone
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    implicitHeight: 32
                    text: "Got it"
                    onClicked: { Settings.hintDone("first-clip"); page.firstClipHint = false }
                }
            }

            VirtualRows {
                id: grid
                anchors.top: hint.visible ? hint.bottom : bar.bottom
                anchors.topMargin: hint.visible ? 16 : 0
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                anchors.right: parent.right
                rows: page.rows
                gap: 20
                base: grid.cardHeight
                smallest: grid.cardHeight
                endSpace: 20 + page.lift

                readonly property int inner: width - 56
                readonly property int columns: Math.max(1, Math.floor((inner + 20) / 400))
                // A card's width goes in steps of 8, and what is left over goes
                // between the cards. So a window being sized mostly moves the
                // cards apart, which costs nothing, and only now and then has
                // every card in view lay out its words and picture anew.
                readonly property int cardWidth: Math.floor((inner - (columns - 1) * 20) / columns / 8) * 8
                readonly property real between: columns > 1 ? (inner - columns * cardWidth) / (columns - 1) : 20
                readonly property int cardHeight: Math.round(cardWidth * 9 / 16) + 54
                // The most cards that have been side by side: each row
                // has that many places, and the ones past `columns` are
                // empty. So a window sized across where one more card
                // fits makes (or lets go of) that one card a row, and
                // not every card in view anew.
                property int most: columns
                onColumnsChanged: { most = Math.max(most, columns); page.layout() }

                // The cards are one stop for Tab; the arrows move among them.
                activeFocusOnTab: true
                onActiveFocusChanged: if (activeFocus && page.current < 0 && page.clips.length > 0) {
                    // Tab came in: start at the first card in view.
                    page.current = page.rows[rowAt(contentY + 60)].first
                    page.byKeys = true
                }
                Accessible.role: Accessible.List
                Accessible.name: "Clips"
                Keys.onPressed: event => {
                    const ctrl = event.modifiers & Qt.ControlModifier
                    const shift = event.modifiers & Qt.ShiftModifier
                    const here = page.current >= 0 ? page.clips[page.current] : null
                    event.accepted = true
                    if (event.key === Qt.Key_Left) { page.go(page.current - 1, shift) }
                    else if (event.key === Qt.Key_Right) { page.go(page.current + 1, shift) }
                    else if (event.key === Qt.Key_Up) { page.go(page.vertical(Math.max(0, page.current), -1), shift) }
                    else if (event.key === Qt.Key_Down) { page.go(page.current < 0 ? 0 : page.vertical(page.current, 1), shift) }
                    else if (event.key === Qt.Key_PageUp) { page.go(page.vertical(Math.max(0, page.current), -2), shift) }
                    else if (event.key === Qt.Key_PageDown) { page.go(page.vertical(Math.max(0, page.current), 2), shift) }
                    else if (event.key === Qt.Key_Home) { page.go(0, shift) }
                    else if (event.key === Qt.Key_End) { page.go(page.clips.length - 1, shift) }
                    else if (event.key === Qt.Key_A && ctrl) { page.setPicked(page.clips.map(c => c.id)) }
                    else if (event.key === Qt.Key_Z && ctrl) { page.undoAsked() }
                    else if (event.key === Qt.Key_Escape && page.pickedCount > 0) { page.clearPicked() }
                    else if (event.key === Qt.Key_Space && here) { page.byKeys = true; page.togglePicked(page.current) }
                    else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter) && here) { page.openClip(here) }
                    else if (event.key === Qt.Key_Delete) { page.trash(page.targets(here)) }
                    else if (event.key === Qt.Key_C && ctrl) { page.copy(page.targets(here)) }
                    else if (event.key === Qt.Key_F && !ctrl) { page.favorite(page.targets(here)) }
                    else if ((event.key === Qt.Key_E && !ctrl) || event.key === Qt.Key_F2) { page.details(page.targets(here)) }
                    else if (event.key === Qt.Key_M && !ctrl) { page.move(page.targets(here)) }
                    else { event.accepted = false }
                }

                delegate: Item {
                    id: line
                    readonly property var row: parent ? parent.row : null
                    readonly property var head: row ? row.head : null
                    anchors.fill: parent

                    // The day or the game, a rule running on from it, and how many.
                    Item {
                        visible: line.head !== null
                        x: 28
                        y: line.head ? line.head.top : 0
                        width: grid.inner
                        height: groupName.implicitHeight
                        Text {
                            id: groupName
                            text: line.head ? line.head.name : ""
                            color: Theme.text
                            font.family: Theme.titleFamily
                            font.pixelSize: 22
                            font.capitalization: Font.AllUppercase
                        }
                        Rectangle {
                            anchors.left: groupName.right
                            anchors.leftMargin: 14
                            anchors.right: groupCount.left
                            anchors.rightMargin: 14
                            anchors.verticalCenter: parent.verticalCenter
                            height: 1
                            color: Theme.line
                        }
                        Text {
                            id: groupCount
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            text: line.head ? page.clipsWord(line.head.count) : ""
                            color: Theme.textFaint
                            font.family: Theme.monoFamily
                            font.pixelSize: 11
                            font.letterSpacing: 1.6
                            font.capitalization: Font.AllUppercase
                        }
                    }
                    Row {
                        x: 28
                        y: line.row ? line.row.headH : 0
                        spacing: grid.between
                        Repeater {
                            model: grid.most
                            Loader {
                                id: place
                                required property int index
                                // Made when first wanted and kept from then on, hidden
                                // while there is no room for it: making a card is the
                                // slow part of a window being sized back and forth.
                                property bool wanted: false
                                readonly property bool fits: index < grid.columns
                                onFitsChanged: if (fits) { wanted = true }
                                Component.onCompleted: wanted = fits
                                active: wanted
                                visible: fits
                                sourceComponent: ClipCard {
                                // Where this card's clip is among the listed ones.
                                readonly property int at: line.row ? line.row.first + place.index : -1
                                entry: line.row && place.index < line.row.clips.length ? line.row.clips[place.index] : null
                                visible: entry !== null
                                cover: entry !== null ? page.covers[entry.game] || "" : ""
                                width: grid.cardWidth
                                above: line.row ? grid.contentY - (line.parent.y + line.row.headH) : 0
                                selected: entry !== null && page.picked[entry.id] === true
                                selecting: page.pickedCount > 0
                                current: entry !== null && page.byKeys && grid.activeFocus && page.current === at
                                Component.onCompleted: page.cardsMade += 1
                                onPressed: modifiers => page.press(at, modifiers)
                                onPicked: { page.byKeys = false; grid.forceActiveFocus(); page.togglePicked(at) }
                                onFavoriteToggled: page.favorite(page.targets(entry))
                                onCopied: page.copy(page.targets(entry))
                                onTrashed: page.trash(page.targets(entry))
                                onDetailsAsked: page.details(page.targets(entry))
                                }
                            }
                        }
                    }
                }
            }

            // How far down the library the view is: a thin mark at the edge.
            Rectangle {
                visible: grid.contentHeight > grid.height
                anchors.right: parent.right
                anchors.rightMargin: 6
                y: grid.y + (grid.height - height) * grid.contentY / Math.max(1, grid.contentHeight - grid.height)
                width: 3
                height: Math.max(28, grid.height * grid.height / grid.contentHeight)
                radius: 2
                color: Theme.textFaint
                opacity: grid.moving ? 0.9 : 0.35
                Behavior on opacity { NumberAnimation { duration: Theme.smooth } }
            }

            SelectionBar {
                anchors.horizontalCenter: grid.horizontalCenter
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 16
                count: page.pickedCount
                allFavorite: page.pickedClips.length > 0 && page.pickedClips.every(c => c.favorite)
                wide: main.width > 760
                onFavorite: page.favorite(page.pickedClips)
                onTag: page.details(page.pickedClips)
                onMove: page.move(page.pickedClips)
                onCopy: page.copy(page.pickedClips)
                onTrash: page.trash(page.pickedClips)
                onCleared: page.clearPicked()
            }

            // Nothing to show: say why, and what to do about it, in an empty
            // viewfinder.
            Item {
                visible: page.clips.length === 0
                anchors.centerIn: grid
                width: empty.implicitWidth + 96
                height: empty.implicitHeight + 80
                Brackets { arm: 22 }
            }
            Column {
                id: empty
                visible: page.clips.length === 0
                anchors.centerIn: grid
                spacing: 10
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: search.text !== "" ? "No clip matches “" + search.text + "”"
                        : page.place === "★" ? "No favourites yet"
                        : "No clips yet"
                    color: Theme.text
                    font.family: Theme.titleFamily
                    font.pixelSize: 30
                    font.capitalization: Font.AllUppercase
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    horizontalAlignment: Text.AlignHCenter
                    text: search.text !== "" ? "Try another word, a tag, or a game's name."
                        : page.place === "★" ? "Star a clip to keep it here."
                        : page.firstWords()
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.body
                    lineHeight: 1.3
                }
                // A recorder that is not running is started from here.
                Btn {
                    visible: search.text === "" && page.place === "" && !page.recorder.running
                    anchors.horizontalCenter: parent.horizontalCenter
                    implicitHeight: 34
                    icon: "play"
                    text: "Start the recorder"
                    onClicked: {
                        const wrong = Settings.startRecorder()
                        if (wrong !== "") { page.say(wrong) }
                        Recorder.poll()
                    }
                }
            }
        }
    }

    // Videos dragged in from a file manager join the library: under the
    // game being looked at, else under "Imported".
    DropArea {
        id: drop
        anchors.fill: parent
        keys: ["text/uri-list"]
        onDropped: dropped => {
            if (!dropped.hasUrls) { return }
            const game = page.place !== "" && page.place !== "★" ? page.place : "Imported"
            Library.importFiles(dropped.urls.map(u => u.toString()).join("\n"), game)
            page.say("Adding to " + game + "…")
            dropped.accept()
        }
    }
    Rectangle {
        anchors.fill: parent
        visible: drop.containsDrag
        color: "#D906090B"
        Item {
            anchors.centerIn: parent
            width: dropWords.implicitWidth + 96
            height: dropWords.implicitHeight + 80
            Brackets { arm: 22 }
            Text {
                id: dropWords
                anchors.centerIn: parent
                text: "Drop to add to the library"
                color: Theme.text
                font.family: Theme.titleFamily
                font.pixelSize: 30
                font.capitalization: Font.AllUppercase
            }
        }
    }

    ClipDetails {
        id: detailsSheet
        onSaved: problem => {
            page.refresh()
            if (problem !== "") { page.say(problem) }
            grid.forceActiveFocus()
        }
        onClosed: grid.forceActiveFocus()
    }
    MoveSheet {
        id: moveSheet
        property var list: []
        onChosen: game => {
            const problem = Library.moveToGame(page.idsOf(list), game)
            page.refresh()
            page.say(problem !== "" ? problem : (list.length === 1 ? "Moved to " : page.clipsWord(list.length) + " moved to ") + game + ".")
            grid.forceActiveFocus()
        }
        onClosed: grid.forceActiveFocus()
    }
    Sheet {
        id: sure
        property var list: []
        title: "Move " + list.length + " clips to the trash?"
        onClosed: grid.forceActiveFocus()
        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "They leave the library and go to the system's trash, where they stay until it is emptied."
            color: Theme.textDim
            font.family: Theme.family
            font.pixelSize: Theme.body
            lineHeight: 1.3
        }
        Row {
            anchors.right: parent.right
            spacing: 10
            Btn { text: "Keep them"; onClicked: sure.close() }
            Btn {
                primary: true
                icon: "trash"
                text: "Move to the trash"
                onClicked: { sure.open = false; page.trashNow(sure.list); grid.forceActiveFocus() }
            }
        }
    }
}

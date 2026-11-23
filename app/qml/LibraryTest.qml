import QtQuick
import Zenith

// `--librarytest <video> [<folder for pictures>]`: use the library the way
// a person would (pick clips, favourite, trash and take it back, sort, drop
// a video in, tag it, rename its file, move it, trash it for good), say on
// the terminal how each step went, and quit. Everything is left as it was:
// the only clip changed for good is the one the test brought in itself.
Item {
    id: test
    // The library page and the window it is in.
    property var page: null
    property var window: null

    readonly property var args: Qt.application.arguments
    readonly property int at: args.indexOf("--librarytest")
    readonly property string video: args[at + 1] || ""
    readonly property string pictures: args.length > at + 2 && args[at + 2].indexOf("--") !== 0 ? args[at + 2] : ""
    property int failed: 0
    property int step: 0
    // The clip the test brought in.
    property var own: null
    property int before: 0

    function check(what, ok) {
        if (!ok) { failed += 1 }
        console.log("librarytest:", ok ? "ok  " : "FAIL", what)
    }
    // A picture of the window as it is once what the step changed has
    // settled (a bar sliding in, a sheet fading up).
    function picture(name) {
        if (pictures === "") { return }
        shutter.name = name
        shutter.restart()
    }
    Timer {
        id: shutter
        property string name: ""
        interval: 800
        onTriggered: test.window.contentItem.grabToImage(result => result.saveToFile(test.pictures + "/" + name + ".png"))
    }
    function mine() { return JSON.parse(Library.clips(JSON.stringify({ search: "library test" }))).find(c => c.game.indexOf("Library test") === 0) }

    // One step at a time, with a moment between them for what a step
    // started (a copy, the trash, a picture) to finish.
    readonly property var steps: [
        () => {
            before = page.clips.length
            check("the library lists its clips (" + before + ")", before >= 6)
            page.go(0, false)
            page.go(page.vertical(0, 1), true)
            check("Shift and Down picks from the first card to the one under it", page.current > 0 && page.pickedCount === page.current + 1)
            page.go(page.clips.length - 1, false)
            const view = page.scroller
            check("End goes to the last clip and scrolls to it (" + Math.round(view.contentY) + " of " + Math.round(view.contentHeight - view.height) + ")",
                page.current === before - 1 && (view.contentHeight <= view.height || view.contentY + view.height >= view.contentHeight - 60))
            page.clearPicked()
            page.scroller.contentY = 0
            page.press(1, Qt.ControlModifier)
            page.press(4, Qt.ShiftModifier)
            check("Ctrl and Shift clicks pick a range of 4", page.pickedCount === 4)
            page.press(2, 0)
            check("a plain click while picking leaves one out", page.pickedCount === 3)
            page.press(2, 0)
            picture("selection")
        },
        () => {
            const were = page.pickedClips.map(c => c.favorite)
            page.favorite(page.pickedClips)
            check("one press favourites all 4", page.pickedCount === 4 && page.pickedClips.every(c => c.favorite))
            page.favorite(page.pickedClips)
            check("and again takes it away", page.pickedClips.every(c => !c.favorite))
            // As they were, for the ones that were favourites before.
            page.pickedClips.forEach((c, i) => Library.setFavorite(c.id, were[i]))
            page.copy(page.pickedClips)
            page.trash(page.pickedClips)
            check("4 clips trashed leave the list", page.clips.length === before - 4 && page.pickedCount === 0)
            picture("undo")
        },
        () => {
            window.undo()
            check("Undo brings all 4 back", page.clips.length === before)
            page.sort = "longest"
            check("longest first", page.clips[0].seconds >= page.clips[page.clips.length - 1].seconds && page.clips[0].seconds >= page.clips[1].seconds)
            page.sort = "smallest"
            check("smallest first", page.clips[0].bytes <= page.clips[page.clips.length - 1].bytes)
            page.grouping = "game"
            picture("sorted")
        },
        () => {
            page.sort = "newest"
            page.grouping = "day"
            // Only a library with that many clips can show it.
            if (before <= page.manyClips) { return }
            page.trash(page.clips.slice(0, page.manyClips + 1))
            check("many clips are asked about, not trashed", page.clips.length === before)
            if (page.clips.length !== before) { window.undo() }
            picture("confirm")
        },
        () => {
            page.closeSheets()
            Library.importFiles(video, "Library test")
        },
        () => {
            own = mine()
            check("a dropped video joins the library under its game", !!own && page.clips.length === before + 1)
            if (!own) { return }
            Library.setTags(own.id, "Test tag, #second")
            Library.setNotes(own.id, "a note with zzzunique in it")
            page.searchText = "zzzunique"
        },
        () => {
            check("a word in the notes finds it", page.clips.length === 1)
            page.searchText = "#second"
        },
        () => {
            check("and so does a tag", page.clips.length === 1 && page.clips[0].tags.join() === "test tag,second")
            page.details(page.clips)
            picture("details")
        },
        () => {
            page.closeSheets()
            page.searchText = ""
            const problem = Library.renameWithFile(own.id, "Library test clip")
            own = mine()
            check("renaming renames the file " + problem, problem === "" && /Library test clip\.[a-z0-9]+$/.test(own.path) && own.title === "Library test clip")
            check("and the tags and notes stay", own.tags.length === 2 && own.notes !== "")
            const moved = Library.moveToGame(JSON.stringify([own.id]), "Library test moved")
            own = mine()
            check("moving takes it to another game " + moved, moved === "" && own.game === "Library test moved" && own.path.indexOf("Library test moved") > 0)
            page.refresh()
            page.move([own])
            picture("move")
        },
        () => {
            page.closeSheets()
            page.trashNow([own])
            window.settleTrash()
        },
        () => {
            page.refresh()
            const still = mine()
            check(still ? "the trash refused the file, and the clip is back in the library" : "its file went to the trash, and the clip is gone", true)
            if (still) { console.log("librarytest: the clip left behind is", still.path) }
            check("everything else is as it was", page.clips.length === before + (still ? 1 : 0))
            console.log("librarytest:", failed === 0 ? "all well" : failed + " failed")
            Qt.exit(failed === 0 ? 0 : 1)
        }
    ]

    Timer {
        interval: 1500
        running: true
        repeat: true
        onTriggered: {
            if (test.step < test.steps.length) { test.steps[test.step++]() }
        }
    }
}

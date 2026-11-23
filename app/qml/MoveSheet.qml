import QtQuick
import Zenith

// Where clips go when they are moved: one of the games there are, or a
// new one by name.
Sheet {
    id: sheet
    property int count: 0
    // [{ game, clips }]
    property var games: []
    signal chosen(string game)

    title: count === 1 ? "Move the clip to" : "Move " + count + " clips to"

    function ask(howMany, shelves) {
        count = howMany
        games = shelves
        fresh.value = ""
        fresh.text = ""
        show()
    }
    function pick(game) {
        if (game.trim() === "") { return }
        open = false
        chosen(game.trim())
    }

    Flickable {
        width: parent.width
        height: Math.min(list.implicitHeight, 264)
        contentHeight: list.implicitHeight
        id: gameList
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        Column {
            id: list
            width: parent.width
            spacing: 2
            Repeater {
                model: sheet.games
                SideItem {
                    required property var modelData
                    label: modelData.game
                    // An icon is drawn as curves, which the list's edge
                    // does not cut: it shows only while all of it is inside.
                    icon: y >= gameList.contentY - 11 && y + height <= gameList.contentY + gameList.height + 11 ? "gamepad" : ""
                    cover: modelData.cover || ""
                    count: modelData.clips
                    onClicked: sheet.pick(modelData.game)
                }
            }
        }
    }
    Row {
        width: parent.width
        spacing: 10
        Field {
            id: fresh
            width: parent.width - move.width - 10
            placeholder: "Or a new game's name"
            onCommitted: words => value = words
            onEntered: sheet.pick(text)
        }
        Btn { id: move; primary: true; text: "Move"; enabled: fresh.text.trim() !== ""; onClicked: sheet.pick(fresh.text) }
    }
}

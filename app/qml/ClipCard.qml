import QtQuick
import Zenith

// One clip in the library: its picture, as large as the card, which
// scrubs through the clip as the pointer moves across it, and under it
// what it is. The library keeps only as many cards as fit the window and
// hands each a new clip as it scrolls, so a card can be without one.
Item {
    id: card
    property var entry: null
    // Picked, as one of the clips the next action is for.
    property bool selected: false
    // Some clip is picked: every card shows where to pick it.
    property bool selecting: false
    // The keyboard is on this card.
    property bool current: false
    // How far the card's top is above the top edge of the list it scrolls
    // in. Icons are drawn as curves, which that edge does not cut, so each
    // is put away once it is past the edge and behind the bar above it.
    property real above: 0
    // Pressed, with the keys held (`Qt.ControlModifier`, `Qt.ShiftModifier`).
    signal pressed(int modifiers)
    signal picked()
    signal favoriteToggled()
    signal copied()
    signal trashed()
    signal detailsAsked()

    readonly property var nothing: ({ id: -1, path: "", game: "", title: "", savedAt: 0, seconds: 0, bytes: 0, favorite: false, trimmed: false, tags: [], notes: "" })
    readonly property var clip: entry || nothing
    readonly property int strip: 12
    property string thumb: ""
    property string film: ""
    // The file the picture was last asked for.
    property string asked: ""
    readonly property bool hovered: area.containsMouse || actions.hovered || tick.hovered

    Accessible.role: Accessible.Button
    Accessible.name: clip.title + ", " + clip.game + ", " + Theme.duration(clip.seconds) + (selected ? ", selected" : "")
    Accessible.onPressAction: card.pressed(0)

    implicitHeight: picture.height + 54

    // The card is headed by the clip's name, with its game under it. A
    // clip nobody named is called after its game and the time it was
    // saved: for a name it has that time, the way one would say it, and
    // not the game twice.
    readonly property bool named: !(clip.title.indexOf(clip.game) === 0 && /\d{4}-\d{2}-\d{2} \d{2}-\d{2}-\d{2}( \(\d+\))?$/.test(clip.title))
    readonly property string heading: named ? clip.title : Theme.day(clip.savedAt) + " " + Theme.clock(clip.savedAt)
    // The game's cover, where it has one: shown small beside its name.
    property string cover: ""

    // The picture is asked for when the card gets a clip. The clip it had
    // is not waited for any more: only cards in view keep ffmpeg busy.
    function load() {
        if (clip.path === asked) { return }
        if (asked !== "" && thumb === "") { Media.thumbnailUnwanted(asked) }
        asked = clip.path
        film = ""
        thumb = asked === "" ? "" : Media.thumbnail(clip.path, clip.seconds)
    }
    // When `clip` changes, not `entry`: by then it is the new clip that
    // `load` reads, never the one the card had.
    onClipChanged: load()
    Component.onCompleted: load()
    Component.onDestruction: if (asked !== "" && thumb === "") { Media.thumbnailUnwanted(asked) }
    onHoveredChanged: if (hovered && film === "" && asked !== "") { film = Media.filmstrip(clip.path, clip.seconds, strip) }

    Connections {
        target: Media
        function onThumbnailReady(path, file) { if (path === card.asked) { card.thumb = file } }
        function onFilmstripReady(path, count, file) { if (path === card.asked && count === card.strip) { card.film = file } }
    }

    Rectangle {
        id: picture
        width: parent.width
        radius: Theme.radius
        height: Math.round(width * 9 / 16)
        color: Theme.surface
        clip: true

        Icon {
            anchors.centerIn: parent
            visible: still.status !== Image.Ready && card.above < picture.height / 2 + 30
            name: "film"
            size: 28
            color: Theme.line
        }

        Image {
            id: still
            anchors.fill: parent
            anchors.margins: 1
            source: Library.fileUrl(card.thumb)
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            sourceSize.width: 640
            opacity: status === Image.Ready ? 1 : 0
            Behavior on opacity { NumberAnimation { duration: Theme.smooth } }
        }

        // The filmstrip, one frame of it shown: the one under the pointer.
        Item {
            anchors.fill: parent
            anchors.margins: 1
            clip: true
            visible: card.hovered && frames.status === Image.Ready
            Image {
                id: frames
                source: Library.fileUrl(card.film)
                asynchronous: true
                height: parent.height
                width: parent.width * card.strip
                fillMode: Image.Stretch
                x: -Math.min(card.strip - 1, Math.max(0, Math.floor(area.mouseX / Math.max(1, area.width) * card.strip))) * parent.width
            }
            Rectangle {
                anchors.bottom: parent.bottom
                height: 3
                width: Math.max(0, Math.min(parent.width, area.mouseX))
                color: Theme.action
            }
        }

        Rectangle {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 8
            width: length.implicitWidth + 12
            height: 22
            radius: 2
            color: "#E6000000"
            Text {
                id: length
                anchors.centerIn: parent
                text: Theme.duration(card.clip.seconds)
                color: "#EEF4F6"
                font.family: Theme.monoFamily
                font.pixelSize: Theme.small
            }
        }

        // Part of the clip is cut away in the editor. (Its volumes alone
        // do not say so.)
        Rectangle {
            visible: !!card.clip.trimmed
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.margins: 8
            width: trimmedLabel.implicitWidth + 12
            height: 22
            radius: 2
            color: Theme.accent
            Text {
                id: trimmedLabel
                anchors.centerIn: parent
                text: "Trimmed"
                color: Theme.accentInk
                font.family: Theme.monoFamily
                font.pixelSize: 11
                font.letterSpacing: 1.2
                font.capitalization: Font.AllUppercase
            }
        }

        MouseArea {
            id: area
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: mouse => card.pressed(mouse.modifiers)
        }

        // Where a clip is picked without holding a key: a box in the
        // corner, there when the pointer is on the card or clips are being
        // picked, and filled in when this one is.
        Rectangle {
            id: tick
            readonly property bool hovered: tickArea.containsMouse
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.margins: 8
            width: 26
            height: 26
            radius: 2
            color: card.selected ? Theme.accent : "#E606090B"
            border.width: card.selected ? 0 : 2
            border.color: tickArea.containsMouse ? Theme.action : Theme.textDim
            opacity: card.hovered || card.selected || card.selecting || card.current ? 1 : 0
            visible: opacity > 0 && card.above < 60
            Behavior on opacity { NumberAnimation { duration: Theme.quick } }
            Icon {
                anchors.centerIn: parent
                visible: card.selected
                name: "check"
                size: 18
                color: Theme.accentInk
            }
            MouseArea {
                id: tickArea
                anchors.fill: parent
                anchors.margins: -4
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: card.picked()
            }
            Tip { shown: tickArea.containsMouse; text: card.selected ? "Leave out" : "Select" }
        }

        Rectangle {
            id: actions
            readonly property bool hovered: rowHover.hovered
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 8
            width: buttons.implicitWidth + 6
            height: 36
            radius: Theme.radius
            color: "#E606090B"
            opacity: card.hovered || card.clip.favorite ? 1 : 0
            visible: opacity > 0 && card.above < 60
            Behavior on opacity { NumberAnimation { duration: Theme.quick } }
            Row {
                id: buttons
                anchors.centerIn: parent
                HoverHandler { id: rowHover }
                // The keyboard reaches these through the library's keys
                // (F, E, Ctrl+C, Delete), not one card at a time with Tab.
                IconButton {
                    activeFocusOnTab: false
                    icon: "star"
                    filled: card.clip.favorite
                    active: card.clip.favorite
                    tip: card.clip.favorite ? "Remove from favourites" : "Favourite"
                    implicitWidth: 30
                    implicitHeight: 30
                    onClicked: card.favoriteToggled()
                }
                IconButton {
                    activeFocusOnTab: false
                    visible: card.hovered
                    icon: "tag"
                    tip: "Name, tags and notes"
                    implicitWidth: 30
                    implicitHeight: 30
                    onClicked: card.detailsAsked()
                }
                IconButton {
                    activeFocusOnTab: false
                    visible: card.hovered
                    icon: "copy"
                    tip: "Copy the file"
                    implicitWidth: 30
                    implicitHeight: 30
                    onClicked: card.copied()
                }
                IconButton {
                    activeFocusOnTab: false
                    visible: card.hovered
                    icon: "trash"
                    tip: "Move to the trash"
                    implicitWidth: 30
                    implicitHeight: 30
                    onClicked: card.trashed()
                }
            }
        }
    }

    // Picked: outlined in the colour of what is chosen. Under the pointer
    // or the keyboard: in the colour of what can be done.
    Rectangle {
        anchors.fill: picture
        anchors.margins: -3
        radius: Theme.radius + 3
        color: "transparent"
        border.width: 2
        border.color: card.current || card.hovered ? Theme.action : Theme.accent
        opacity: card.current || card.hovered || card.selected ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.quick } }
    }

    Text {
        id: title
        anchors.top: picture.bottom
        anchors.topMargin: 8
        width: parent.width
        text: card.heading
        textFormat: Text.PlainText
        color: Theme.text
        elide: Text.ElideRight
        font.family: Theme.titleFamily
        font.pixelSize: 20
        font.capitalization: Font.AllUppercase
    }
    // The game: its cover, or a gamepad for one without, then its name.
    Item {
        id: mark
        anchors.left: parent.left
        anchors.verticalCenter: facts.verticalCenter
        width: 13
        height: 17
        // Icons are curves, which the list's top edge does not cut.
        visible: card.above < picture.height + 40
        Icon {
            anchors.centerIn: parent
            visible: card.cover === ""
            name: "gamepad"
            size: 13
            color: Theme.textFaint
        }
        Rectangle {
            visible: card.cover !== ""
            anchors.fill: parent
            radius: 2
            color: Theme.surface
            clip: true
            Image {
                anchors.fill: parent
                source: Library.fileUrl(card.cover)
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                sourceSize.height: 40
            }
        }
    }
    Text {
        id: facts
        anchors.top: title.bottom
        anchors.topMargin: 2
        anchors.left: mark.right
        anchors.leftMargin: 6
        width: parent.width - mark.width - 6 - (tags.visible ? tags.width + 10 : 0)
        // The day and time are the heading of a clip without a name.
        text: card.clip.game + (card.named ? "  " + Theme.day(card.clip.savedAt) + " " + Theme.clock(card.clip.savedAt) : "") + "  " + Theme.size(card.clip.bytes)
        color: Theme.textFaint
        elide: Text.ElideRight
        font.family: Theme.monoFamily
        font.pixelSize: 11
        font.letterSpacing: 0.6
        font.capitalization: Font.AllUppercase
    }
    // Its tags, at the end of the same line.
    Text {
        id: tags
        visible: card.clip.tags.length > 0
        anchors.right: parent.right
        anchors.baseline: facts.baseline
        width: Math.min(implicitWidth, parent.width * 0.5)
        text: card.clip.tags.map(t => "#" + t).join(" ")
        textFormat: Text.PlainText
        color: Theme.accentWords
        elide: Text.ElideRight
        font.family: Theme.monoFamily
        font.pixelSize: 11
        font.letterSpacing: 0.6
    }
}

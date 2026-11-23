import QtQuick
import Zenith

// Beside the picture, under export: what the clip is, the few things done
// with it that have no place on the timeline, and the library's other
// clips, to go on to without leaving the editor. Cutting, the sound and
// going through the clip are all done on the timeline and by its keys.
// In a tall window all of it is bigger, to be easier to read and to hit.
Item {
    id: panel
    property var entry: null
    // The editor's state, for whether anything is changed.
    property var edit: null

    signal say(string message)
    // The frame that is showing is to be kept as a picture.
    signal frameWanted()
    // Another clip is to be opened in this one's place.
    signal clipWanted(var clip)

    // Kept among the favourites: the library reads it again when the editor closes.
    property bool favorite: entry ? !!entry.favorite : false
    // The library's clips, newest first.
    property var clips: []
    Component.onCompleted: clips = JSON.parse(Library.clips("{}"))

    // A tall panel has room: its lines, words and buttons are bigger.
    readonly property bool tall: height >= 620
    readonly property int row: tall ? 40 : 30
    readonly property int pad: 14

    component Act: Btn { width: (panel.width - 2 * panel.pad - 8) / 2; implicitHeight: panel.tall ? 40 : 32 }

    Column {
        id: own
        width: parent.width
        PanelTitle { text: "Clip" }
        Item { width: 1; height: 10 }
        // What it is, on a card as a group of settings is. The game and
        // the day are over the picture already, and what is kept is
        // beside the playhead's time.
        Rectangle {
            x: panel.pad
            width: parent.width - 2 * panel.pad
            height: facts.height
            radius: Theme.radiusLarge
            color: Theme.surface
            border.color: Theme.line
            Column {
                id: facts
                width: parent.width
        Repeater {
            model: panel.entry ? [
                ["Picture", panel.entry.width + "×" + panel.entry.height + "  " + Math.round(panel.entry.fps) + " fps"],
                ["Length", Theme.timecode(panel.entry.seconds)],
                ["On disk", Theme.size(panel.entry.bytes)]
            ] : []
            Item {
                id: fact
                required property var modelData
                required property int index
                width: facts.width
                height: panel.row
                Rectangle { visible: fact.index > 0; x: 1; width: parent.width - 2; height: 1; color: Theme.line; opacity: 0.6 }
                Text {
                    x: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: fact.modelData[0]
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: panel.tall ? Theme.body : Theme.small
                }
                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: fact.modelData[1]
                    textFormat: Text.PlainText
                    color: Theme.text
                    font.family: Theme.monoFamily
                    font.pixelSize: panel.tall ? 14 : 12
                    font.features: { "tnum": 1 }
                }
            }
        }
            }
        }
        Item {
            width: parent.width
            height: acts.height + 10
            Grid {
                id: acts
                x: panel.pad
                y: 10
                columns: 2
                spacing: 8
                Act {
                    icon: "star"
                    text: panel.favorite ? "Favourite ✓" : "Favourite"
                    tip: panel.favorite ? "Take it out of the favourites" : "Keep it among the favourites"
                    onClicked: { panel.favorite = !panel.favorite; Library.setFavorite(panel.entry.id, panel.favorite) }
                }
                Act { icon: "camera"; text: "Save frame"; tip: "Keep the frame that is showing as a PNG, beside the clip and on the clipboard"; onClicked: panel.frameWanted() }
                Act { icon: "copy"; text: "Copy file"; tip: "Copy the clip as recorded, to paste as a file"; onClicked: { Library.copyToClipboard(panel.entry.path); panel.say("Copied the clip as recorded.") } }
                Act { icon: "folder"; text: "Show folder"; tip: "Show its folder"; onClicked: Qt.openUrlExternally(Library.fileUrl(panel.entry.path.replace(/[\\/][^\\/]*$/, ""))) }
            }
        }
        PanelTitle { text: "Library" }
        Item { width: 1; height: 8 }
    }

    // The other clips, in what room is left.
    ListView {
        id: feed
        anchors.top: own.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: panel.clips
        readonly property int here: panel.entry ? panel.clips.findIndex(c => c.id === panel.entry.id) : -1
        onHereChanged: if (here >= 0) { positionViewAtIndex(here, ListView.Center) }
        delegate: Rectangle {
            id: other
            required property var modelData
            required property int index
            readonly property bool on: index === feed.here
            property string thumb: ""
            x: 8
            width: feed.width - 16
            height: panel.tall ? 64 : 50
            radius: Theme.radius
            color: on ? Theme.raised : otherArea.containsMouse ? Theme.surface : "transparent"
            Behavior on color { ColorAnimation { duration: Theme.quick } }
            Component.onCompleted: thumb = Media.thumbnail(modelData.path, modelData.seconds)
            Component.onDestruction: if (thumb === "") { Media.thumbnailUnwanted(modelData.path) }
            Connections {
                target: Media
                function onThumbnailReady(path, file) { if (path === other.modelData.path) { other.thumb = file } }
            }
            activeFocusOnTab: true
            Keys.onPressed: event => {
                if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (!other.on) { panel.clipWanted(other.modelData) }
                    event.accepted = true
                }
            }
            Accessible.role: Accessible.ListItem
            Accessible.name: modelData.title + ", " + modelData.game + ", " + Theme.duration(modelData.seconds) + (on ? ", open" : "")
            Accessible.onPressAction: if (!other.on) { panel.clipWanted(other.modelData) }
            Rectangle { visible: other.on; width: 3; height: 18; radius: 2; color: Theme.accent; anchors.verticalCenter: parent.verticalCenter }
            MouseArea {
                id: otherArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: other.on ? Qt.ArrowCursor : Qt.PointingHandCursor
                onClicked: if (!other.on) { panel.clipWanted(other.modelData) }
            }
            Rectangle {
                id: shot
                x: 10
                anchors.verticalCenter: parent.verticalCenter
                height: parent.height - 12
                width: Math.round(height * 16 / 9)
                radius: 2
                color: "#000000"
                clip: true
                Image {
                    anchors.fill: parent
                    source: Library.fileUrl(other.thumb)
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    opacity: status === Image.Ready ? 1 : 0
                    Behavior on opacity { NumberAnimation { duration: Theme.smooth } }
                }
            }
            Column {
                anchors.left: shot.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                Text {
                    width: parent.width
                    elide: Text.ElideRight
                    text: other.modelData.title
                    textFormat: Text.PlainText
                    color: other.on ? Theme.text : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: panel.tall ? 13 : 12
                    font.weight: Font.DemiBold
                }
                Text {
                    width: parent.width
                    elide: Text.ElideRight
                    text: Theme.duration(other.modelData.seconds) + "  ·  " + Theme.day(other.modelData.savedAt) + " " + Theme.clock(other.modelData.savedAt)
                    textFormat: Text.PlainText
                    color: Theme.textFaint
                    font.family: Theme.family
                    font.pixelSize: 11
                }
            }
            FocusRing { anchors.margins: -1; radius: other.radius + 1 }
        }
    }
}

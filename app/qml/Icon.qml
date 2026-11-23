import QtQuick
import QtQuick.Shapes

// A line icon, drawn on a 24 grid. `filled` fills it in (a set star, play).
Item {
    id: icon
    property string name: ""
    property color color: Theme.textDim
    property bool filled: false
    property real size: 18
    implicitWidth: size
    implicitHeight: size

    readonly property var paths: ({
        "search": "M11 4a7 7 0 1 0 0 14a7 7 0 0 0 0-14z M20 20l-4-4",
        "star": "M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8L3.5 9.7l5.9-.8z",
        "play": "M7.5 4.5v15l12-7.5z",
        "pause": "M8 5v14 M16 5v14",
        "restart": "M6 5v14 M19 5.5v13l-9.5-6.5z",
        "trash": "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v6 M14 11v6",
        "copy": "M9 9h11v11H9z M5 15H4V4h11v1",
        "folder": "M3 6h6l2 2h10v11H3z",
        "settings": "M10.5 5.8 L10.9 3.5 L13.1 3.5 L13.5 5.8 L15.3 6.5 L17.2 5.2 L18.8 6.8 L17.5 8.7 L18.2 10.5 L20.5 10.9 L20.5 13.1 L18.2 13.5 L17.5 15.3 L18.8 17.2 L17.2 18.8 L15.3 17.5 L13.5 18.2 L13.1 20.5 L10.9 20.5 L10.5 18.2 L8.7 17.5 L6.8 18.8 L5.2 17.2 L6.5 15.3 L5.8 13.5 L3.5 13.1 L3.5 10.9 L5.8 10.5 L6.5 8.7 L5.2 6.8 L6.8 5.2 L8.7 6.5z M12 9.3a2.7 2.7 0 1 0 0 5.4a2.7 2.7 0 0 0 0-5.4z",
        "back": "M15 5l-7 7 7 7",
        "split": "M12 3.5v17 M8 8.5L4.5 12 8 15.5 M16 8.5l3.5 3.5-3.5 3.5",
        "left": "M14 7l-5 5 5 5",
        "right": "M10 7l5 5-5 5",
        "left2": "M12 7l-5 5 5 5 M18 7l-5 5 5 5",
        "right2": "M6 7l5 5-5 5 M12 7l5 5-5 5",
        "startHere": "M9 5H6v14h3 M11 12h8",
        "endHere": "M15 5h3v14h-3 M5 12h8",
        "solo": "M5 14v-2a7 7 0 0 1 14 0v2 M5 14h3v5H5z M16 14h3v5h-3z",
        "camera": "M4 8h3.5l1.8-2.5h5.4L16.5 8H20v11H4z M12 10.5a3 3 0 1 0 0 6a3 3 0 0 0 0-6z",
        "prevCut": "M7 6v12 M17 7l-6 5 6 5",
        "nextCut": "M17 6v12 M7 7l6 5-6 5",
        "first": "M5 6v12 M8.5 6v12 M19 7l-6 5 6 5",
        "last": "M19 6v12 M15.5 6v12 M5 7l6 5-6 5",
        "chevron": "M7 10l5 5 5-5",
        "link": "M14 5h5v5 M19 5l-8 8 M17 14v5H5V7h5",
        "scissors": "M6 4a2.5 2.5 0 1 0 0 5a2.5 2.5 0 0 0 0-5z M6 15a2.5 2.5 0 1 0 0 5a2.5 2.5 0 0 0 0-5z M8 8l12 10 M8 16L20 6",
        "undo": "M4 9h10a5 5 0 0 1 0 10H9 M8 5L4 9l4 4",
        "redo": "M20 9H10a5 5 0 0 0 0 10h5 M16 5l4 4-4 4",
        "volume": "M4 9h4l5-4v14l-5-4H4z M16.5 9a4.5 4.5 0 0 1 0 6",
        "mute": "M4 9h4l5-4v14l-5-4H4z M16.5 9.5l4.5 5 M21 9.5l-4.5 5",
        "check": "M5 12.5l4.5 4.5L19 7",
        "x": "M6 6l12 12 M18 6L6 18",
        "export": "M12 4v11 M7 10.5l5 5 5-5 M5 20h14",
        "grid": "M4 4h7v7H4z M13 4h7v7h-7z M4 13h7v7H4z M13 13h7v7h-7z",
        "clock": "M12 3.5a8.5 8.5 0 1 0 0 17a8.5 8.5 0 0 0 0-17z M12 8v4.5l3 2",
        "gamepad": "M7 8h10a4 4 0 0 1 4 4v1a3 3 0 0 1-5.5 1.5L15 14H9l-.5.5A3 3 0 0 1 3 13v-1a4 4 0 0 1 4-4z M8 10.5v3 M6.5 12h3",
        "edit": "M4 20l1-4L16 5l3 3L8 19z M14 7l3 3",
        "film": "M4 5h16v14H4z M8 5v14 M16 5v14 M4 9.5h4 M4 14.5h4 M16 9.5h4 M16 14.5h4",
        "plus": "M12 5v14 M5 12h14",
        "minus": "M5 12h14",
        "square": "M6 6h12v12H6z",
        "squares": "M6 9h9v9H6z M9 9V6h9v9h-3",
        "refresh": "M19 12a7 7 0 1 1-2.5-5.4 M19 4v4h-4",
        "tag": "M4 4h8l8 8-8 8-8-8z M8.5 8.5h.01",
        "sort": "M4 7h16 M4 12h11 M4 17h6",
        "move": "M3 6h6l2 2h10v11H3z M9 13.5h6 M12.5 11l2.5 2.5-2.5 2.5"
    })

    Shape {
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: icon.size / 24
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: icon.color
            strokeWidth: 1.8
            fillColor: icon.filled ? icon.color : "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.paths[icon.name] || "" }
        }
    }
}

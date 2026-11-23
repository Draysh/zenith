import QtQuick

// The four corner marks of a viewfinder, around whatever it fills.
Item {
    id: marks
    property color tint: Theme.accent
    property int arm: 16
    property int weight: 2
    anchors.fill: parent
    Repeater {
        model: [[0, 0], [1, 0], [0, 1], [1, 1]]
        Item {
            id: corner
            required property var modelData
            x: modelData[0] ? marks.width - marks.arm : 0
            y: modelData[1] ? marks.height - marks.arm : 0
            width: marks.arm
            height: marks.arm
            Rectangle { width: marks.arm; height: marks.weight; y: corner.modelData[1] ? marks.arm - marks.weight : 0; color: marks.tint }
            Rectangle { width: marks.weight; height: marks.arm; x: corner.modelData[0] ? marks.arm - marks.weight : 0; color: marks.tint }
        }
    }
}

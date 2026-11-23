import QtQuick
import Zenith

// The setup checks: each thing clipping needs, whether it is there, and
// what to do when it is not.
Column {
    id: list
    // `worst` is "checking" until the first answer is in.
    property var report: ({ worst: "checking", checks: [] })
    function refresh() { Settings.doctorStart() }
    Connections {
        target: Settings
        function onDoctorReady(report) { list.report = JSON.parse(report) }
    }
    Component.onCompleted: refresh()
    width: parent ? parent.width : 0

    Repeater {
        model: list.report.checks
        Item {
            id: row
            required property var modelData
            required property int index
            width: list.width
            height: Math.max(52, words.implicitHeight + 22)

            Rectangle {
                id: mark
                x: 18
                y: 15
                width: 22
                height: 22
                radius: Theme.radius
                color: row.modelData.level === "ok" ? "#266FE08A" : row.modelData.level === "warning" ? "#33FFB454" : "#33FF5A3C"
                Icon {
                    anchors.centerIn: parent
                    size: 13
                    name: row.modelData.level === "ok" ? "check" : row.modelData.level === "warning" ? "minus" : "x"
                    color: row.modelData.level === "ok" ? Theme.goodWords : row.modelData.level === "warning" ? Theme.warnWords : Theme.record
                }
            }
            Column {
                id: words
                anchors.left: mark.right
                anchors.leftMargin: 14
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                spacing: 3
                Text {
                    width: parent.width
                    text: row.modelData.name
                    color: Theme.text
                    font.family: Theme.family
                    font.pixelSize: Theme.body
                    font.weight: Font.Medium
                }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: row.modelData.detail + (row.modelData.fix ? "  " + row.modelData.fix : "")
                    color: row.modelData.level === "ok" ? Theme.textFaint : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.small
                    lineHeight: 1.25
                }
            }
            Rectangle {
                visible: row.index < list.report.checks.length - 1
                anchors.bottom: parent.bottom
                x: 18
                width: parent.width - 36
                height: 1
                color: Theme.line
                opacity: 0.5
            }
        }
    }
}

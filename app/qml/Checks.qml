import QtQuick
import Zenith

// The setup checks: each thing clipping needs, whether it is there, and
// what to do when it is not.
Column {
    id: list
    // `worst` is "checking" until the first answer is in.
    property var report: ({ worst: "checking", checks: [] })
    // Only what is wrong, and not what is fine (the welcome's first step).
    property bool troubleOnly: false
    readonly property var shown: troubleOnly ? report.checks.filter(c => c.level === "problem") : report.checks
    signal say(string message)
    function refresh() { Settings.doctorStart() }
    // Started, the recorder takes a moment to answer: asked again after it.
    Timer { id: again; interval: 1500; onTriggered: list.refresh() }
    Connections {
        target: Settings
        function onDoctorReady(report) { list.report = JSON.parse(report) }
    }
    Component.onCompleted: refresh()
    width: parent ? parent.width : 0

    Repeater {
        model: list.shown
        Item {
            id: row
            required property var modelData
            required property int index
            width: list.width
            height: Math.max(52, words.implicitHeight + 22)
            // A thing Zenith can put right itself gets a button for it,
            // in place of a command to type.
            readonly property bool startable: modelData.name === "Background recorder" && modelData.level !== "ok"

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
                anchors.right: startIt.visible ? startIt.left : parent.right
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
                    text: row.modelData.detail + (row.modelData.fix && !row.startable ? "  " + row.modelData.fix : "")
                    color: row.modelData.level === "ok" ? Theme.textFaint : Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: Theme.small
                    lineHeight: 1.25
                }
            }
            Btn {
                id: startIt
                visible: row.startable
                anchors.right: parent.right
                anchors.rightMargin: 18
                anchors.verticalCenter: parent.verticalCenter
                implicitHeight: 32
                icon: "play"
                text: "Start it"
                onClicked: {
                    const wrong = Settings.startRecorder()
                    if (wrong !== "") { list.say(wrong) }
                    again.restart()
                }
            }
            Rectangle {
                visible: row.index < list.shown.length - 1
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

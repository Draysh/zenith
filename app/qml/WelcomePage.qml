import QtQuick
import Zenith

// The first time the app is opened: what it does, whether this machine is
// ready, and the one thing only the user can do, which is to pick a key.
Rectangle {
    id: page
    signal closed()
    signal say(string message)
    color: Theme.bg

    Frit { anchors.top: parent.top; anchors.right: parent.right; width: 520; height: 240 }

    Flickable {
        anchors.fill: parent
        contentHeight: body.implicitHeight + 96
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: body
            y: 56
            width: Math.min(640, parent.width - 48)
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 28

            Column {
                width: parent.width
                spacing: 12
                Logo { size: 56; z: 1 }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: "Keep the moment that just happened"
                    color: Theme.accentWords
                    font.family: Theme.titleFamily
                    font.pixelSize: 52
                    font.capitalization: Font.AllUppercase
                    lineHeight: 0.8
                }
                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    text: "While a game is running, Zenith remembers the last minute. Press your shortcut and that minute is saved as a clip, ready to trim and share."
                    color: Theme.textDim
                    font.family: Theme.family
                    font.pixelSize: 15
                    lineHeight: 1.35
                }
            }

            SettingGroup {
                title: checks.report.worst === "checking" ? "Checking this computer" : checks.report.worst === "ok" ? "This computer is ready" : checks.report.worst === "warning" ? "Ready, with one thing to know" : "Something needs fixing first"
                Checks { id: checks }
            }

            // On Windows the recorder listens for the key itself.
            SettingGroup {
                visible: Qt.platform.os === "windows"
                title: "Your clip key"
                SettingRow {
                    label: "Alt+F9 saves a clip"
                    note: "Press it anywhere, in a game too. Another key can be chosen in the settings."
                }
            }

            SettingGroup {
                visible: Qt.platform.os !== "windows"
                title: "Pick your shortcut"
                SettingRow {
                    label: "Bind a key to this command"
                    note: "In your desktop's keyboard settings, add a custom shortcut (Alt+S is a good one) that runs it."
                    stacked: true
                    Rectangle {
                        width: parent.width
                        height: 40
                        radius: Theme.radius
                        color: Theme.bg
                        border.color: Theme.line
                        Text {
                            anchors.left: parent.left
                            anchors.leftMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            text: Settings.saveCommand()
                            elide: Text.ElideMiddle
                            width: parent.width - 56
                            color: Theme.text
                            font.family: Theme.monoFamily
                            font.pixelSize: Theme.body
                        }
                        IconButton {
                            anchors.right: parent.right
                            anchors.rightMargin: 4
                            anchors.verticalCenter: parent.verticalCenter
                            icon: "copy"
                            tip: "Copy the command"
                            onClicked: { Settings.copyText(Settings.saveCommand()); page.say("Copied.") }
                        }
                    }
                }
            }

            Row {
                spacing: 10
                Btn {
                    primary: true
                    implicitHeight: 42
                    text: "Open my clips"
                    onClicked: { Settings.welcomeDone(); page.closed() }
                }
                Btn {
                    implicitHeight: 42
                    icon: "refresh"
                    text: "Check again"
                    onClicked: checks.refresh()
                }
            }
        }
    }
}

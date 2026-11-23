pragma Singleton
import QtQuick
import Zenith

// The look in one place: colours, sizes, type, and how numbers are worded.
QtObject {
    // Surfaces, from the window back to what sits on top: a blue-black,
    // the dark of a screen that is switched off.
    // The background and the lines can be the user's own too (`look` in
    // the settings). The surfaces on a background of their own are steps
    // from it towards white (towards black on a light one), and on a light
    // one the words are dark.
    property string ownBg: ""
    property string ownLine: ""
    readonly property string bgDefault: "#06090B"
    readonly property string lineDefault: "#25333D"
    function luma(c) { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b }
    readonly property bool light: luma(bg) > 0.35
    function step(amount) { return Qt.tint(bg, light ? Qt.rgba(0, 0, 0, amount) : Qt.rgba(1, 1, 1, amount)) }
    readonly property color bg: ownBg !== "" ? ownBg : bgDefault
    readonly property color surface: ownBg !== "" ? step(0.04) : "#0D1317"
    readonly property color raised: ownBg !== "" ? step(0.08) : "#151D23"
    readonly property color hover: ownBg !== "" ? step(0.13) : "#1E2A32"
    readonly property color line: ownLine !== "" ? ownLine : ownBg !== "" ? step(0.17) : lineDefault

    readonly property color text: light ? "#0C1114" : "#EEF4F6"
    readonly property color textDim: light ? "#39464D" : "#A9B8BF"
    readonly property color textFaint: light ? "#56636A" : "#8496A0"

    // The two colours that speak can be the user's own (`look` in the
    // settings): "#RRGGBB", or "" for Zenith's. What goes with each (its
    // lighter shade under the pointer, the ink written on it) follows.
    property string ownAccent: ""
    property string ownAction: ""
    readonly property string accentDefault: "#3DE0F5"
    readonly property string actionDefault: "#FF3DA5"
    function use(look) {
        ownAccent = look && look.accent ? look.accent : ""
        ownAction = look && look.action ? look.action : ""
        ownBg = look && look.background ? look.background : ""
        ownLine = look && look.line ? look.line : ""
        iconSoon.restart()
    }
    // The app's icon is the mark in the same colours (see `useIcon` in the
    // settings). A moment after the last change: a colour being picked
    // changes many times a second.
    property Timer iconSoon: Timer {
        interval: 400
        onTriggered: Settings.useIcon("" + accent, "" + accentInk, "" + action)
    }
    Component.onCompleted: use(JSON.parse(Settings.load()).look)
    function lighter(c) { return Qt.tint(c, "#4DFFFFFF") }
    // Dark ink on a light colour, light ink on a dark one.
    function ink(c) { return luma(c) > 0.35 ? Qt.tint(c, "#E6000000") : "#FFFFFF" }

    // Cyan says what is: the name of the place, what is chosen, the part
    // of a clip that is kept.
    readonly property color accent: ownAccent !== "" ? ownAccent : accentDefault
    readonly property color accentHover: ownAccent !== "" ? lighter(accent) : "#7BEBFA"
    readonly property color accentInk: ownAccent !== "" ? ink(accent) : "#02161A"
    // The same colours for words and marks drawn straight on the window:
    // on a light window a light colour is darkened until it can be read.
    function onWindow(c) { return light && luma(c) > 0.3 ? Qt.darker(c, 1.9) : c }
    readonly property color accentWords: onWindow(accent)
    // Magenta says what can be done: the main button, a switch, a slider,
    // a handle, where the keyboard is.
    readonly property color action: ownAction !== "" ? ownAction : actionDefault
    readonly property color actionHover: ownAction !== "" ? lighter(action) : "#FF6CBB"
    readonly property color actionInk: ownAction !== "" ? ink(action) : "#22000F"
    readonly property color record: "#FF5A3C"
    readonly property color warn: "#FFB454"
    readonly property color good: "#6FE08A"
    readonly property color goodWords: onWindow(good)
    readonly property color warnWords: onWindow(warn)

    // Paper: the label on a tape. Clips and lists to read sit on it.
    readonly property color paper: "#F1EFE4"
    readonly property color paperInk: "#0C1114"
    readonly property color paperQuiet: "#56636A"
    readonly property color paperEdge: "#D8D5C6"
    readonly property color paperChosen: ownAccent !== "" ? Qt.tint(paper, Qt.rgba(accent.r, accent.g, accent.b, 0.3)) : "#C4EEF3"

    readonly property int radius: 4
    readonly property int radiusLarge: 6
    readonly property int gap: 8
    readonly property int pad: 16

    // The typefaces come with the app. Inter for reading, Anton (tall,
    // in capitals) for the names of things, Plex Mono for times and sizes.
    readonly property FontLoader interFont: FontLoader { source: "qrc:/qt/qml/Zenith/fonts/Inter.ttf" }
    readonly property FontLoader antonFont: FontLoader { source: "qrc:/qt/qml/Zenith/fonts/Anton-Regular.ttf" }
    readonly property FontLoader monoFont: FontLoader { source: "qrc:/qt/qml/Zenith/fonts/IBMPlexMono-Regular.ttf" }
    readonly property FontLoader monoMediumFont: FontLoader { source: "qrc:/qt/qml/Zenith/fonts/IBMPlexMono-Medium.ttf" }
    readonly property string family: interFont.status === FontLoader.Ready ? interFont.name : Qt.application.font.family
    readonly property string titleFamily: antonFont.status === FontLoader.Ready ? antonFont.name : family
    readonly property string monoFamily: monoFont.status === FontLoader.Ready ? monoFont.name : "monospace"
    readonly property int small: 12
    readonly property int body: 14
    readonly property int title: 18
    readonly property int display: 26

    readonly property int quick: 120
    readonly property int smooth: 220

    // 75.4 -> "1:15"
    function duration(seconds) {
        const s = Math.max(0, Math.round(seconds))
        const m = Math.floor(s / 60)
        const h = Math.floor(m / 60)
        const two = n => (n < 10 ? "0" : "") + n
        return h > 0 ? h + ":" + two(m % 60) + ":" + two(s % 60) : m + ":" + two(s % 60)
    }

    // 75.432 -> "1:15.43", for the editor
    function timecode(seconds) {
        const s = Math.max(0, seconds)
        const m = Math.floor(s / 60)
        const rest = s - m * 60
        return m + ":" + (rest < 10 ? "0" : "") + rest.toFixed(2)
    }

    function size(bytes) {
        const mb = bytes / (1024 * 1024)
        if (mb >= 1024) {
            return (mb / 1024).toFixed(1) + " GB"
        }
        return (mb >= 10 ? Math.round(mb) : mb.toFixed(1)) + " MB"
    }

    // The day a clip was saved, the way one would say it.
    function day(epochSeconds) {
        const then = new Date(epochSeconds * 1000)
        const start = d => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
        const days = Math.round((start(new Date()) - start(then)) / 86400000)
        if (days === 0) {
            return "Today"
        }
        if (days === 1) {
            return "Yesterday"
        }
        if (days < 7) {
            return then.toLocaleDateString(Qt.locale(), "dddd")
        }
        const sameYear = then.getFullYear() === new Date().getFullYear()
        return then.toLocaleDateString(Qt.locale(), sameYear ? "d MMMM" : "d MMMM yyyy")
    }

    function clock(epochSeconds) {
        return new Date(epochSeconds * 1000).toLocaleTimeString(Qt.locale(), "HH:mm")
    }
}

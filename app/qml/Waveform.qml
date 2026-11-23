import QtQuick

// An audio track's shape, as bars. Only the part in view is drawn: the
// item is as wide as the window onto the track, `total` is how wide the
// whole track is at this zoom, and `offset` how far the view has moved.
//
// While the window is being sized the bars are not drawn again at every
// step: what is drawn is stretched to the width of the moment, so it
// moves in the same picture as everything around it, and is drawn anew
// once the width rests.
Item {
    id: wave
    property var peaks: []
    property color tint: Theme.textDim
    // How loud the track is set: the bars grow and shrink with it.
    property real gain: 1
    property real total: width
    property real offset: 0
    // The width the bars were last drawn for, and whether it is on its way
    // to another.
    property real drawn: 0
    property bool sizing: false

    function draw() { drawn = width; paper.requestPaint() }
    // What comes with a new width (the track's whole width, where the
    // view is) waits for the width to rest; a zoom or a scroll does not.
    function moved() { Qt.callLater(() => { if (!wave.sizing) { paper.requestPaint() } }) }

    onPeaksChanged: paper.requestPaint()
    onTintChanged: paper.requestPaint()
    onGainChanged: paper.requestPaint()
    onHeightChanged: paper.requestPaint()
    onTotalChanged: moved()
    onOffsetChanged: moved()
    onWidthChanged: {
        if (drawn < 1) { draw(); return }
        sizing = true
        rest.restart()
    }
    Timer { id: rest; interval: 140; onTriggered: { wave.sizing = false; wave.draw() } }

    Canvas {
        id: paper
        width: wave.drawn
        height: wave.height
        transform: Scale { xScale: wave.drawn > 0 ? wave.width / wave.drawn : 1 }

        onPaint: {
            const ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            if (!wave.peaks || wave.peaks.length === 0) {
                return
            }
            ctx.fillStyle = wave.tint
            const step = 3
            // The bars are counted along the whole track, so they stay where
            // they are as the view moves.
            const bars = Math.max(1, Math.floor(wave.total / step))
            const first = Math.max(0, Math.floor(wave.offset / step))
            const last = Math.min(bars, Math.ceil((wave.offset + width) / step))
            const mid = height / 2
            for (let i = first; i < last; i++) {
                const from = Math.floor(i * wave.peaks.length / bars)
                const to = Math.max(from + 1, Math.floor((i + 1) * wave.peaks.length / bars))
                let peak = 0
                for (let j = from; j < to && j < wave.peaks.length; j++) {
                    peak = Math.max(peak, wave.peaks[j])
                }
                // The square root lifts quiet parts so they can be seen.
                const h = Math.max(1.5, Math.min(1, Math.sqrt(peak) * Math.min(wave.gain, 1.4)) * (height - 4))
                ctx.fillRect(i * step - wave.offset, mid - h / 2, 2, h)
            }
        }
    }
}

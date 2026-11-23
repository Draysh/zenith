import QtQuick

// A barcode made from a piece of text: the same text always gives the
// same bars. Only to look at; nothing reads it.
Canvas {
    id: bars
    property string code: ""
    property color tint: Theme.text
    implicitWidth: 150
    implicitHeight: 34
    onCodeChanged: requestPaint()
    onTintChanged: requestPaint()
    onWidthChanged: requestPaint()
    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        ctx.fillStyle = tint
        // A small generator seeded by the text decides each bar's width.
        let seed = 2166136261
        for (let i = 0; i < code.length; i++) { seed = Math.imul(seed ^ code.charCodeAt(i), 16777619) >>> 0 }
        let x = 0
        while (x < width) {
            seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0
            const bar = 1 + (seed >>> 28) % 3
            const gap = 1 + (seed >>> 24) % 3
            ctx.fillRect(x, 0, Math.min(bar, width - x), height)
            x += bar + gap
        }
    }
}

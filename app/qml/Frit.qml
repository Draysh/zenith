import QtQuick

// A field of dots that thins out away from its top right corner, the way
// a printed picture breaks into dots at its edge. Only to look at.
Canvas {
    id: frit
    property color tint: Theme.accent
    property int pitch: 12
    implicitWidth: 420
    implicitHeight: 190
    opacity: 0.75
    onTintChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        ctx.fillStyle = tint
        const reach = Math.hypot(width, height)
        for (let y = pitch / 2; y < height; y += pitch) {
            for (let x = pitch / 2; x < width; x += pitch) {
                // Full size in the corner, nothing at the far edge.
                const far = Math.hypot(width - x, y * 1.7) / reach
                const r = (pitch / 2 - 1) * Math.pow(Math.max(0, 1 - far * 1.25), 2.2)
                if (r > 0.35) {
                    ctx.beginPath()
                    ctx.arc(x, y, r, 0, Math.PI * 2)
                    ctx.fill()
                }
            }
        }
    }
}

import QtQuick
import QtQuick.Shapes

// Zenith's mark: a Z on the cyan the app names things in, and above its
// corner a magenta dot: the highest point, and a light that says it is
// recording. The same drawing as the app's icon (app/icons/zenith.svg), on
// its 256 grid.
Item {
    id: logo
    property real size: 24
    implicitWidth: size
    implicitHeight: size

    Item {
        width: 256
        height: 256
        scale: logo.size / 256
        transformOrigin: Item.TopLeft
        Rectangle { anchors.fill: parent; radius: 52; color: Theme.accent }
        Rectangle { x: 60; y: 50; width: 136; height: 36; color: Theme.accentInk }
        Rectangle { x: 60; y: 170; width: 136; height: 36; color: Theme.accentInk }
        Shape {
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: "transparent"
                fillColor: Theme.accentInk
                PathSvg { path: "M150 86L196 86L106 170L60 170Z" }
            }
        }
        // The dot, with a ring of the tile's colour so it stands clear of the Z.
        Rectangle { x: 184; y: 24; width: 48; height: 48; radius: 24; color: Theme.accent }
        Rectangle { x: 188; y: 28; width: 40; height: 40; radius: 20; color: Theme.action }
    }
}

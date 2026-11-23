import QtQuick

// A long list of rows of which only the ones in view exist. However many
// rows there are, as many are made as fit the view; a row scrolled out of
// sight is handed the next row coming into sight, so scrolling makes
// nothing new and the list opens as fast with two thousand rows as with
// twenty.
Flickable {
    id: view
    // The rows, top to bottom. Every row is `base` tall, and `h` more
    // where it says so (a heading over it). The two are apart so that the
    // rows need not be made again when only their common height changes,
    // as it does at every step of the window being sized.
    property var rows: []
    property real base: 0
    onBaseChanged: place()
    // What draws a row. It fills its parent and reads the row to show from
    // `parent.row`, which is one of `rows` or null when it has none.
    property Component delegate
    // The space between two rows, and the room kept free under the last.
    property real gap: 0
    property real endSpace: 40
    // The shortest a row can be: how many are kept ready follows from it.
    property real smallest: 100
    // Rows are made ready this far before they scroll into sight.
    property real ahead: 300

    // Where each row begins; one more entry, for where the last one ends.
    property var tops: [0]
    onRowsChanged: place()
    function place() {
        const starts = new Array(rows.length + 1)
        let y = 0
        for (let i = 0; i < rows.length; i++) {
            starts[i] = y
            y += base + rows[i].h + gap
        }
        starts[rows.length] = y
        tops = starts
        // A list that got shorter is not left scrolled past its end.
        returnToBounds()
    }

    // The row that `y` is in.
    function rowAt(y) {
        let low = 0
        let high = rows.length - 1
        while (low < high) {
            const middle = (low + high + 1) >> 1
            if (tops[middle] <= y) { low = middle } else { high = middle - 1 }
        }
        return Math.max(0, low)
    }

    // Scroll no further than needed for row `index` to be all in view.
    function reveal(index) {
        if (index < 0 || index >= rows.length) { return }
        const top = tops[index]
        if (top < contentY) {
            contentY = Math.max(0, top - gap)
        } else if (top + base + rows[index].h > contentY + height - endSpace) {
            // Clear of whatever lies over the foot of the list.
            contentY = top + base + rows[index].h - height + endSpace
        }
    }

    readonly property int first: rowAt(contentY - ahead)
    readonly property int slots: Math.max(1, Math.ceil((height + 2 * ahead) / (smallest + gap)) + 1)

    contentHeight: tops[tops.length - 1] + endSpace
    boundsBehavior: Flickable.StopAtBounds
    clip: true

    Repeater {
        model: view.slots
        Loader {
            required property int index
            // Each row in view belongs to the one slot whose number is the
            // row's remainder: moving on by a row changes a single slot.
            readonly property int rowIndex: view.first + ((index - view.first) % view.slots + view.slots) % view.slots
            readonly property var row: rowIndex < view.rows.length ? view.rows[rowIndex] : null
            y: row ? (view.tops[rowIndex] || 0) : 0
            width: view.width
            height: row ? view.base + row.h : 0
            // A row waiting out of sight is ready, and not drawn.
            visible: row !== null && y + height > view.contentY && y < view.contentY + view.height
            sourceComponent: view.delegate
        }
    }
}

// The few Qt calls the Rust side has no binding for.

#pragma once

#include <QtCore/QFile>
#include <QtCore/QList>
#include <QtCore/QMimeData>
#include <QtCore/QString>
#include <QtCore/QUrl>
#include <QtGui/QClipboard>
#include <QtGui/QColor>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtGui/QImage>
#include <QtGui/QPainter>
#include <QtGui/QPixmap>
#include <QtGui/QPolygonF>
#include <QtCore/QElapsedTimer>
#include <QtCore/QMutex>
#include <QtCore/QTimer>
#include <QtGui/QOpenGLContext>
#include <QtGui/QOpenGLFunctions>
#include <QtGui/QScreen>
#include <QtQuick/QQuickWindow>

#include <cstdio>
#if defined(Q_OS_LINUX)
#include <csignal>
#include <execinfo.h>
#include <unistd.h>
#endif

#ifdef Q_OS_WIN
#include <QtCore/QAbstractNativeEventFilter>
#include <QtCore/QCoreApplication>
#include <QtCore/QRectF>
#include <QtCore/QVariant>
#include <QtCore/qt_windows.h>
#include <QtGui/QWindow>
#include <io.h>
#include <windowsx.h>
#endif

namespace cct {

#ifdef Q_OS_WIN
// What Windows asks the app's window, answered so that the window is all
// the app's own to draw, bar included, and still one of Windows' own kind.
//
// The button that fills the screen is the app's own drawing, but Windows
// is told that is what it is (HTMAXBUTTON), which is what brings up
// Windows 11's choice of places to snap the window to when the pointer
// rests on it. A place Windows takes for a button of the frame's is no
// longer the app's to hear clicks and the pointer on, so those are taken
// here and passed on: the click fills the screen or puts the window back,
// and the window is told when the pointer is on the button and when it is
// held down (`maxHovered`, `maxPressed` in Main.qml), to draw it so.
// Where the button is, is the window's to say (`maxButton` there).
class FrameFilter : public QAbstractNativeEventFilter {
public:
  FrameFilter(HWND window, QWindow* shown)
    : m_window(window)
    , m_shown(shown)
  {
  }

  // Whether a place on the screen is on the button that fills the screen.
  bool onMaxButton(LPARAM place) const
  {
    const QRectF button = m_shown->property("maxButton").toRectF();
    if (!button.isValid()) {
      return false;
    }
    POINT at = { GET_X_LPARAM(place), GET_Y_LPARAM(place) };
    if (!ScreenToClient(m_window, &at)) {
      return false;
    }
    // The window counts in its own points; Windows in the screen's dots.
    const qreal dots = m_shown->devicePixelRatio();
    return QRectF(button.x() * dots, button.y() * dots, button.width() * dots, button.height() * dots).contains(at.x, at.y);
  }

  void say(const char* what, bool& known, bool now)
  {
    if (known != now) {
      known = now;
      m_shown->setProperty(what, now);
    }
  }

  bool nativeEventFilter(const QByteArray& type, void* message, qintptr* result) override
  {
    if (type != "windows_generic_MSG") {
      return false;
    }
    const MSG* asked = static_cast<const MSG*>(message);
    if (asked->hwnd != m_window) {
      return false;
    }
    // A message is shown here twice: as it is taken off the queue, when
    // there is nowhere to put an answer (`result` is null), and as the
    // window is handed it. Clicks and the pointer come by the queue, so
    // an answer is only written where there is a place for it.
    const auto answer = [result](qintptr with) {
      if (result) {
        *result = with;
      }
      return true;
    };
    switch (asked->message) {
    // What is at a place: the button that fills the screen, where it is.
    case WM_NCHITTEST:
      if (onMaxButton(asked->lParam)) {
        return answer(HTMAXBUTTON);
      }
      say("maxHovered", m_hovered, false);
      say("maxPressed", m_pressed, false);
      return false;
    // The pointer on that button, and off the frame again.
    case WM_NCMOUSEMOVE:
      if (asked->wParam == HTMAXBUTTON) {
        say("maxHovered", m_hovered, true);
        TRACKMOUSEEVENT leaving = {};
        leaving.cbSize = sizeof(leaving);
        leaving.dwFlags = TME_LEAVE | TME_NONCLIENT;
        leaving.hwndTrack = m_window;
        TrackMouseEvent(&leaving);
      }
      return false;
    case WM_NCMOUSELEAVE:
      say("maxHovered", m_hovered, false);
      say("maxPressed", m_pressed, false);
      return false;
    // A click on it. Windows would draw a frame's button being pressed.
    case WM_NCLBUTTONDOWN:
    case WM_NCLBUTTONDBLCLK:
      if (asked->wParam != HTMAXBUTTON) {
        return false;
      }
      say("maxPressed", m_pressed, true);
      return answer(0);
    case WM_NCLBUTTONUP:
      if (asked->wParam != HTMAXBUTTON) {
        return false;
      }
      if (m_pressed) {
        say("maxPressed", m_pressed, false);
        // Asked of Windows the way its own button asks, and done once
        // this message is over: sizing the window from in here would
        // have Qt lay out and draw the whole window anew inside the
        // answer to a click.
        PostMessageW(m_window, WM_SYSCOMMAND, IsZoomed(m_window) ? SC_RESTORE : SC_MAXIMIZE, 0);
      }
      return answer(0);
    // How much of the window is the frame's: none of it.
    case WM_NCCALCSIZE:
      if (asked->wParam) {
        return answer(0);
      }
      return false;
    // The frame is told the window is in front or not, without drawing
    // its title bar, which would flash over the app's own.
    case WM_NCACTIVATE:
      if (!result) {
        return false;
      }
      return answer(DefWindowProcW(asked->hwnd, WM_NCACTIVATE, asked->wParam, -1));
    // Filling the screen: a window with a frame is made larger than the
    // screen by the frame's width, which this one has none of to hide
    // there. It fills what the taskbar leaves, exactly. (The smallest
    // size is still Qt's to add, so this is not the last word.)
    case WM_GETMINMAXINFO: {
      MONITORINFO monitor = {};
      monitor.cbSize = sizeof(monitor);
      if (GetMonitorInfoW(MonitorFromWindow(m_window, MONITOR_DEFAULTTONEAREST), &monitor)) {
        MINMAXINFO* sizes = reinterpret_cast<MINMAXINFO*>(asked->lParam);
        sizes->ptMaxPosition.x = monitor.rcWork.left - monitor.rcMonitor.left;
        sizes->ptMaxPosition.y = monitor.rcWork.top - monitor.rcMonitor.top;
        sizes->ptMaxSize.x = monitor.rcWork.right - monitor.rcWork.left;
        sizes->ptMaxSize.y = monitor.rcWork.bottom - monitor.rcWork.top;
      }
      return false;
    }
    default:
      return false;
    }
  }

private:
  HWND m_window;
  QWindow* m_shown;
  bool m_hovered = false;
  bool m_pressed = false;
};
#endif

// The program going down: said in the log before it does, with where it
// was and how it got there, as the name of each program file and how far
// into it. That is what there is to go on afterwards, on a computer that
// is someone else's.
#ifdef Q_OS_WIN
inline void sayWhere(void* address)
{
  HMODULE module = nullptr;
  wchar_t name[MAX_PATH] = L"";
  if (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, static_cast<LPCWSTR>(address), &module)
      && module && GetModuleFileNameW(module, name, MAX_PATH)) {
    const wchar_t* file = wcsrchr(name, L'\\');
    fprintf(stderr, "  %ls+0x%llx\n", file ? file + 1 : name, static_cast<unsigned long long>(static_cast<char*>(address) - reinterpret_cast<char*>(module)));
  } else {
    fprintf(stderr, "  %p\n", address);
  }
}

inline LONG WINAPI wentDown(EXCEPTION_POINTERS* what)
{
  // Said once: Windows may come by here again for the same fall.
  static bool said = false;
  if (said) {
    return EXCEPTION_CONTINUE_SEARCH;
  }
  said = true;
  const EXCEPTION_RECORD* record = what->ExceptionRecord;
  fprintf(stderr, "==== crashed: exception 0x%08lx at\n", static_cast<unsigned long>(record->ExceptionCode));
  sayWhere(record->ExceptionAddress);
  if (record->ExceptionCode == EXCEPTION_ACCESS_VIOLATION && record->NumberParameters >= 2) {
    fprintf(stderr, "  %s the address 0x%llx\n", record->ExceptionInformation[0] ? "writing to" : "reading", static_cast<unsigned long long>(record->ExceptionInformation[1]));
  }
  void* frames[40];
  const USHORT count = CaptureStackBackTrace(0, 40, frames, nullptr);
  fprintf(stderr, "  called from\n");
  for (USHORT i = 0; i < count; ++i) {
    sayWhere(frames[i]);
  }
  fflush(stderr);
  // On to whatever Windows does with it next.
  return EXCEPTION_CONTINUE_SEARCH;
}
#elif defined(Q_OS_LINUX)
inline void wentDown(int signal)
{
  // Only what may be called from here: no formatting, no memory asked for.
  const char said[] = "==== crashed: signal ";
  const char number[] = { static_cast<char>('0' + signal / 10 % 10), static_cast<char>('0' + signal % 10), '\n' };
  ssize_t done = write(STDERR_FILENO, said, sizeof(said) - 1);
  done = write(STDERR_FILENO, number, sizeof(number));
  (void)done;
  void* frames[40];
  backtrace_symbols_fd(frames, backtrace(frames, 40), STDERR_FILENO);
  // And down, the way it was going.
  std::signal(signal, SIG_DFL);
  raise(signal);
}
#endif

// From here on, what the program says (Qt's messages and QML's, the Rust
// side's, a crash) is added to the file at `path`, which is where it can
// be found again after the window is gone: Windows shows a program with a
// window no stderr at all. Started from a terminal the words stay there,
// and nothing is kept. Whether it is kept in the file.
inline bool keepLog(const QString& path)
{
  bool kept = false;
#ifdef Q_OS_WIN
  if (_wfreopen(reinterpret_cast<const wchar_t*>(path.utf16()), L"a", stderr)) {
    setvbuf(stderr, nullptr, _IONBF, 0);
    // Rust writes to what Windows calls the error output, not to C's.
    SetStdHandle(STD_ERROR_HANDLE, reinterpret_cast<HANDLE>(_get_osfhandle(_fileno(stderr))));
    kept = true;
  }
  SetUnhandledExceptionFilter(wentDown);
#elif defined(Q_OS_LINUX)
  if (!isatty(STDERR_FILENO) && freopen(QFile::encodeName(path).constData(), "a", stderr)) {
    setvbuf(stderr, nullptr, _IONBF, 0);
    kept = true;
  }
  // The list of where it was is read once now, so it is in memory later.
  void* warm[1];
  backtrace(warm, 1);
  for (int signal : { SIGSEGV, SIGABRT, SIGBUS, SIGFPE, SIGILL }) {
    std::signal(signal, wentDown);
  }
#else
  Q_UNUSED(path);
#endif
  return kept;
}

// On Windows, the app's window has no frame of the system's (the bar at
// the top is the app's own), which by itself would also cost it what
// Windows gives a framed window: its shadow and rounded corners, snapping
// to the sides of the screen (dragged there, or Win and an arrow key), and
// the way it grows and shrinks when it is minimised. So the window is
// given a frame's style back, and Windows is told that the frame takes up
// no room (see `FrameFilter`). After the window is made; does nothing
// elsewhere, or when started with `--system-frame`.
inline void ownFrame()
{
#ifdef Q_OS_WIN
  if (QCoreApplication::arguments().contains(QStringLiteral("--system-frame"))) {
    return;
  }
  const auto windows = QGuiApplication::topLevelWindows();
  for (QWindow* window : windows) {
    if (!(window->flags() & Qt::FramelessWindowHint)) {
      continue;
    }
    const HWND handle = reinterpret_cast<HWND>(window->winId());
    // Lives as long as the program.
    QCoreApplication::instance()->installNativeEventFilter(new FrameFilter(handle, window));
    const LONG_PTR style = GetWindowLongPtrW(handle, GWL_STYLE);
    SetWindowLongPtrW(handle, GWL_STYLE, style | WS_THICKFRAME | WS_CAPTION | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU);
    // Has Windows ask about the frame again, with the answers above.
    SetWindowPos(handle, nullptr, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    break;
  }
#endif
}

// How drawing went while the window was being sized, said in the log once
// the sizing is over: how many pictures were drawn, how many of them took
// long enough to be seen as a hitch, and where the slowest one's time
// went. A hitch on someone else's computer is otherwise only a word.
struct Sizing {
  QMutex lock;
  QElapsedTimer clock;
  bool on = false;
  // Of the picture being made: when each part of it began, in ms.
  qint64 swapped = 0, syncing = 0, synced = 0, drawing = 0, drawn = 0;
  qint64 began = 0;
  int frames = 0, slow = 0, slower = 0;
  // The slowest picture: all of it, then taking over what the window
  // holds, drawing it, and putting it on the screen.
  qint64 worst = 0, worstSync = 0, worstDraw = 0, worstShow = 0;
};

// After the window is made.
inline void watchSizing()
{
  const auto windows = QGuiApplication::topLevelWindows();
  for (QWindow* shown : windows) {
    auto* window = qobject_cast<QQuickWindow*>(shown);
    if (!window) {
      continue;
    }
    // Both live as long as the program.
    auto* sizing = new Sizing;
    sizing->clock.start();
    auto* over = new QTimer(window);
    over->setSingleShot(true);
    over->setInterval(700);
    const auto sized = [sizing, over] {
      QMutexLocker held(&sizing->lock);
      if (!sizing->on) {
        sizing->on = true;
        sizing->began = sizing->clock.elapsed();
        sizing->swapped = sizing->began;
        sizing->frames = sizing->slow = sizing->slower = 0;
        sizing->worst = sizing->worstSync = sizing->worstDraw = sizing->worstShow = 0;
      }
      over->start();
    };
    QObject::connect(window, &QWindow::widthChanged, window, sized);
    QObject::connect(window, &QWindow::heightChanged, window, sized);
    QObject::connect(over, &QTimer::timeout, window, [sizing, window] {
      QMutexLocker held(&sizing->lock);
      sizing->on = false;
      const qint64 lasted = sizing->clock.elapsed() - sizing->began - 700;
      // A window put somewhere in one go is not a sizing by hand.
      if (sizing->frames < 5) {
        return;
      }
      fprintf(stderr,
        "sizing: %lld ms to %dx%d, %d pictures, %d over 33 ms, %d over 100 ms; the slowest took %lld ms (taking over %lld, drawing %lld, showing %lld, before those %lld)\n",
        static_cast<long long>(lasted), window->width(), window->height(), sizing->frames, sizing->slow, sizing->slower,
        static_cast<long long>(sizing->worst), static_cast<long long>(sizing->worstSync), static_cast<long long>(sizing->worstDraw),
        static_cast<long long>(sizing->worstShow), static_cast<long long>(sizing->worst - sizing->worstSync - sizing->worstDraw - sizing->worstShow));
    });
    // These are said by whichever thread draws.
    const auto mark = [sizing](qint64 Sizing::* part) {
      return [sizing, part] {
        QMutexLocker held(&sizing->lock);
        sizing->*part = sizing->clock.elapsed();
      };
    };
    QObject::connect(window, &QQuickWindow::beforeSynchronizing, window, mark(&Sizing::syncing), Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::afterSynchronizing, window, mark(&Sizing::synced), Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::beforeRendering, window, mark(&Sizing::drawing), Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::afterRendering, window, mark(&Sizing::drawn), Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::frameSwapped, window, [sizing] {
      QMutexLocker held(&sizing->lock);
      const qint64 now = sizing->clock.elapsed();
      const qint64 took = now - sizing->swapped;
      sizing->swapped = now;
      if (!sizing->on) {
        return;
      }
      sizing->frames += 1;
      sizing->slow += took > 33;
      sizing->slower += took > 100;
      if (took > sizing->worst) {
        sizing->worst = took;
        sizing->worstSync = sizing->synced - sizing->syncing;
        sizing->worstDraw = sizing->drawn - sizing->drawing;
        sizing->worstShow = now - sizing->drawn;
      }
    }, Qt::DirectConnection);
    // What draws, once: a hitch on one graphics card is not one on another.
    QObject::connect(window, &QQuickWindow::sceneGraphInitialized, window, [window] {
      QOpenGLContext* context = QOpenGLContext::currentContext();
      if (!context) {
        return;
      }
      const QScreen* screen = window->screen();
      fprintf(stderr, "drawing with %s, OpenGL %s; the screen shows %.0f pictures a second\n",
        reinterpret_cast<const char*>(context->functions()->glGetString(GL_RENDERER)),
        reinterpret_cast<const char*>(context->functions()->glGetString(GL_VERSION)), screen ? screen->refreshRate() : 0.0);
    }, Qt::DirectConnection);
    break;
  }
}

// The window's icon (the taskbar's, the window switcher's) as Zenith's
// mark in the given colours: the tile, the Z on it, the dot. The same
// drawing as app/icons/zenith.svg, on its 256 grid.
inline void useIcon(const QString& tile, const QString& ink, const QString& dot)
{
  QImage image(256, 256, QImage::Format_ARGB32_Premultiplied);
  image.fill(Qt::transparent);
  {
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    painter.setBrush(QColor(tile));
    painter.drawRoundedRect(QRectF(0, 0, 256, 256), 52, 52);
    painter.setBrush(QColor(ink));
    painter.drawRect(QRectF(60, 50, 136, 36));
    painter.drawRect(QRectF(60, 170, 136, 36));
    painter.drawPolygon(QPolygonF({ QPointF(150, 86), QPointF(196, 86), QPointF(106, 170), QPointF(60, 170) }));
    painter.setBrush(QColor(tile));
    painter.drawEllipse(QPointF(208, 48), 24, 24);
    painter.setBrush(QColor(dot));
    painter.drawEllipse(QPointF(208, 48), 16, 16);
  }
  QGuiApplication::setWindowIcon(QIcon(QPixmap::fromImage(image)));
}

// The video item draws with OpenGL, so the whole scene must. Before the
// application object is made.
inline void useOpenGl()
{
  QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
}

// Put files on the clipboard the way a file manager does, so they can be
// pasted into a chat as uploads. `paths` is one path to a line.
inline void copyFiles(const QString& paths)
{
  QList<QUrl> urls;
  QByteArray gnome("copy");
  for (const QString& path : paths.split(QLatin1Char('\n'), Qt::SkipEmptyParts)) {
    const QUrl url = QUrl::fromLocalFile(path);
    urls.append(url);
    gnome += '\n' + url.toEncoded();
  }
  if (urls.isEmpty()) {
    return;
  }
  auto* data = new QMimeData;
  data->setUrls(urls);
  data->setData(QStringLiteral("x-special/gnome-copied-files"), gnome);
  data->setText(paths);
  QGuiApplication::clipboard()->setMimeData(data);
}

// A file's address, with every character a path can hold (#, %, spaces)
// written the way a URL needs it; and back from such an address to a path.
inline QString fileUrl(const QString& path)
{
  return path.isEmpty() ? QString() : QString::fromUtf8(QUrl::fromLocalFile(path).toEncoded());
}

inline QString localPath(const QString& url)
{
  const QUrl parsed(url);
  return parsed.isLocalFile() ? parsed.toLocalFile() : url;
}

// Move a file to the system's trash (the Recycle Bin), from where it can
// be brought back.
inline bool moveToTrash(const QString& path)
{
  return QFile::moveToTrash(path);
}

inline void copyText(const QString& text)
{
  QGuiApplication::clipboard()->setText(text);
}

} // namespace cct

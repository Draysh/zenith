#include "mpvitem.h"

#include <QtCore/QByteArray>
#include <QtCore/QMetaObject>
#include <QtCore/QMutex>
#include <QtCore/QMutexLocker>
#include <QtGui/QGuiApplication>
#include <QtGui/QOpenGLContext>
#include <QtOpenGL/QOpenGLFramebufferObject>
#include <QtQuick/QQuickOpenGLUtils>
#include <QtQuick/QQuickWindow>

#include <clocale>
#include <mpv/client.h>
#include <mpv/render_gl.h>

// The player and its drawing context, shared by the item (GUI thread) and
// its renderer (render thread). Everything in it is touched under `lock`.
//
// The drawing context is the renderer's to free: mpv wants that done where
// the OpenGL context it was made in is current, which is the render thread,
// and the player must still be there when it is. So whichever of the two
// goes last ends the player:
//  - the renderer first (or none ever made): it frees the drawing context,
//    if it made one, and the item ends the player when it goes;
//  - the item first, with a drawing context still there: it leaves the
//    player behind (`orphaned`), and the renderer frees the context and
//    then ends the player;
//  - the item first, with no drawing context: it ends the player itself,
//    and a renderer that comes or goes after finds nothing to do.
struct MpvShared
{
  QMutex lock;
  MpvItem* item = nullptr;
  mpv_handle* mpv = nullptr;
  mpv_render_context* gl = nullptr;
  // The item is gone and has left `mpv` to the renderer to end.
  bool orphaned = false;
};

namespace {

void* procAddress(void*, const char* name)
{
  QOpenGLContext* gl = QOpenGLContext::currentContext();
  return gl ? reinterpret_cast<void*>(gl->getProcAddress(QByteArray(name))) : nullptr;
}

class MpvRenderer : public QQuickFramebufferObject::Renderer
{
public:
  explicit MpvRenderer(std::shared_ptr<MpvShared> shared) : m_shared(std::move(shared)) {}

  // Qt destroys a renderer on the render thread with the OpenGL context
  // current: the one place the drawing context may be freed.
  ~MpvRenderer() override
  {
    QMutexLocker held(&m_shared->lock);
    if (m_shared->gl) {
      mpv_render_context_free(m_shared->gl);
      m_shared->gl = nullptr;
    }
    // The item went before: the player was left for here, after its context.
    if (m_shared->orphaned && m_shared->mpv) {
      mpv_terminate_destroy(m_shared->mpv);
      m_shared->mpv = nullptr;
    }
  }

  QOpenGLFramebufferObject* createFramebufferObject(const QSize& size) override
  {
    QMutexLocker held(&m_shared->lock);
    if (m_shared->mpv && m_shared->item && !m_shared->gl) {
      mpv_opengl_init_params gl{ procAddress, nullptr };
      // The display, so mpv can hand decoded frames to the GPU without a copy.
      mpv_render_param display{ MPV_RENDER_PARAM_INVALID, nullptr };
#if defined(Q_OS_LINUX)
      if (auto* wayland = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>()) {
        display = { MPV_RENDER_PARAM_WL_DISPLAY, wayland->display() };
      } else if (auto* x11 = qGuiApp->nativeInterface<QNativeInterface::QX11Application>()) {
        display = { MPV_RENDER_PARAM_X11_DISPLAY, x11->display() };
      }
#endif
      mpv_render_param params[]{
        { MPV_RENDER_PARAM_API_TYPE, const_cast<char*>(MPV_RENDER_API_TYPE_OPENGL) },
        { MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, &gl },
        display,
        { MPV_RENDER_PARAM_INVALID, nullptr },
      };
      if (mpv_render_context_create(&m_shared->gl, m_shared->mpv, params) < 0) {
        m_shared->gl = nullptr;
        qWarning("zenith-clips: mpv could not draw with OpenGL");
        QMetaObject::invokeMethod(m_shared->item, "renderFailed", Qt::QueuedConnection);
      } else {
        // mpv has a new frame: ask Qt to draw the item again.
        mpv_render_context_set_update_callback(
          m_shared->gl,
          [](void* ctx) { QMetaObject::invokeMethod(static_cast<MpvItem*>(ctx), "update", Qt::QueuedConnection); },
          m_shared->item);
        QMetaObject::invokeMethod(m_shared->item, "renderReady", Qt::QueuedConnection);
      }
    }
    return QQuickFramebufferObject::Renderer::createFramebufferObject(size);
  }

  void render() override
  {
    QMutexLocker held(&m_shared->lock);
    if (!m_shared->gl) {
      return;
    }
    QOpenGLFramebufferObject* fbo = framebufferObject();
    mpv_opengl_fbo target{ static_cast<int>(fbo->handle()), fbo->width(), fbo->height(), 0 };
    int flip = 0;
    mpv_render_param params[]{
      { MPV_RENDER_PARAM_OPENGL_FBO, &target },
      { MPV_RENDER_PARAM_FLIP_Y, &flip },
      { MPV_RENDER_PARAM_INVALID, nullptr },
    };
    mpv_render_context_render(m_shared->gl, params);
    QQuickOpenGLUtils::resetOpenGLState();
  }

private:
  std::shared_ptr<MpvShared> m_shared;
};

} // namespace

MpvItem::MpvItem(QQuickItem* parent) : QQuickFramebufferObject(parent), m_shared(std::make_shared<MpvShared>())
{
  m_shared->item = this;
  // mpv reads numbers in the C locale and refuses to start in another.
  std::setlocale(LC_NUMERIC, "C");
  // Qt shows the frame buffer the way mpv fills it; no mirroring is needed.
  m_mpv = mpv_create();
  if (!m_mpv) {
    qWarning("zenith-clips: mpv could not be created");
    // Said once the page is listening.
    QMetaObject::invokeMethod(this, [this] { emit failed(QStringLiteral("the video player could not be started")); }, Qt::QueuedConnection);
    return;
  }
  const char* options[][2] = {
    { "config", "no" },          // the user's mpv.conf is for mpv, not for this
    { "terminal", "no" },
    { "vo", "libmpv" },
    { "hwdec", "auto-safe" },
    { "keep-open", "yes" },      // stay on the last frame at the end
    { "pause", "yes" },
    { "hr-seek", "yes" },
    { "osc", "no" },
    { "input-default-bindings", "no" },
    { "input-vo-keyboard", "no" },
    { "audio-client-name", "Zenith" },
    { "volume-max", "100" },
    // The clip and nothing else: no scripts, no fetching from the network,
    // no files lying beside it loaded along.
    { "load-scripts", "no" },
    { "ytdl", "no" },
    { "sub-auto", "no" },
    { "audio-file-auto", "no" },
    { "cover-art-auto", "no" },
  };
  for (const auto& option : options) {
    mpv_set_option_string(m_mpv, option[0], option[1]);
  }
  // ZENITH_MPV_LOG=1 prints what mpv has to say, for finding out why
  // something does not play.
  if (qEnvironmentVariableIsSet("ZENITH_MPV_LOG")) {
    mpv_set_option_string(m_mpv, "terminal", "yes");
    mpv_set_option_string(m_mpv, "msg-level", "all=v");
  }
  if (mpv_initialize(m_mpv) < 0) {
    qWarning("zenith-clips: mpv could not start");
    mpv_destroy(m_mpv);
    m_mpv = nullptr;
    QMetaObject::invokeMethod(this, [this] { emit failed(QStringLiteral("the video player could not be started")); }, Qt::QueuedConnection);
    return;
  }
  mpv_observe_property(m_mpv, 0, "time-pos", MPV_FORMAT_DOUBLE);
  mpv_observe_property(m_mpv, 0, "duration", MPV_FORMAT_DOUBLE);
  mpv_observe_property(m_mpv, 0, "pause", MPV_FORMAT_FLAG);
  mpv_observe_property(m_mpv, 0, "eof-reached", MPV_FORMAT_FLAG);
  mpv_set_wakeup_callback(
    m_mpv,
    [](void* ctx) { QMetaObject::invokeMethod(static_cast<MpvItem*>(ctx), "drainEvents", Qt::QueuedConnection); },
    this);
  QMutexLocker held(&m_shared->lock);
  m_shared->mpv = m_mpv;
}

MpvItem::~MpvItem()
{
  // Under the lock: a frame being drawn finishes first, and the renderer
  // does not go at the same moment.
  QMutexLocker held(&m_shared->lock);
  m_shared->item = nullptr;
  if (!m_mpv) {
    return;
  }
  // Nothing of mpv's calls this item from here on.
  mpv_set_wakeup_callback(m_mpv, nullptr, nullptr);
  if (m_shared->gl) {
    // The drawing context can only be freed on the render thread, and the
    // player must outlive it: both are the renderer's to end now (see
    // `MpvShared`). Until it does, nothing plays and nothing is heard.
    mpv_render_context_set_update_callback(m_shared->gl, nullptr, nullptr);
    const char* stop[] = { "stop", nullptr };
    mpv_command_async(m_mpv, 0, stop);
    m_shared->orphaned = true;
  } else {
    m_shared->mpv = nullptr;
    mpv_terminate_destroy(m_mpv);
  }
  m_mpv = nullptr;
}

QQuickFramebufferObject::Renderer* MpvItem::createRenderer() const
{
  window()->setPersistentGraphics(true);
  window()->setPersistentSceneGraph(true);
  return new MpvRenderer(m_shared);
}

void MpvItem::command(const QStringList& args)
{
  if (!m_mpv) {
    return;
  }
  QList<QByteArray> bytes;
  for (const QString& arg : args) {
    bytes.append(arg.toUtf8());
  }
  QList<const char*> raw;
  for (const QByteArray& b : bytes) {
    raw.append(b.constData());
  }
  raw.append(nullptr);
  mpv_command_async(m_mpv, 0, raw.data());
}

void MpvItem::setSource(const QString& path)
{
  if (path == m_source) {
    return;
  }
  m_source = path;
  // Another file: its mix is built for the tracks it has.
  if (m_mpv && m_mixed > 0) {
    mpv_set_property_string(m_mpv, "af", "");
    mpv_set_property_string(m_mpv, "lavfi-complex", "");
  }
  m_mixed = -1;
  if (m_loaded) {
    m_loaded = false;
    emit loadedChanged();
  }
  load();
  emit sourceChanged();
}

// A file opened before mpv can draw would play without a picture, so
// loading waits for the first frame buffer.
void MpvItem::load()
{
  if (m_source.isEmpty()) {
    command({ "stop" });
  } else if (m_canDraw) {
    command({ "loadfile", m_source });
  }
}

void MpvItem::renderFailed()
{
  emit failed(QStringLiteral("the picture could not be drawn (no OpenGL)"));
}

// Also after a renderer went and another came (the window let go of its
// graphics): freeing the drawing context ended the picture of the file
// that was open, so it is opened again.
void MpvItem::renderReady()
{
  if (!m_canDraw) {
    m_canDraw = true;
    emit canDrawChanged();
  }
  load();
}

void MpvItem::setPaused(bool paused)
{
  if (!m_mpv) {
    return;
  }
  int flag = paused ? 1 : 0;
  mpv_set_property_async(m_mpv, 0, "pause", MPV_FORMAT_FLAG, &flag);
}

void MpvItem::setVolume(double volume)
{
  volume = qBound(0.0, volume, 1.0);
  if (qFuzzyCompare(volume + 1, m_volume + 1)) {
    return;
  }
  m_volume = volume;
  if (m_mpv) {
    double percent = volume * 100.0;
    mpv_set_property_async(m_mpv, 0, "volume", MPV_FORMAT_DOUBLE, &percent);
  }
  emit volumeChanged();
}

void MpvItem::seek(double seconds, bool exact)
{
  command({ "seek", QString::number(qMax(0.0, seconds), 'f', 4), exact ? "absolute+exact" : "absolute+keyframes" });
}

void MpvItem::step(int frames)
{
  command({ frames < 0 ? "frame-back-step" : "frame-step" });
}

void MpvItem::setTrackGains(const QVariantList& gains)
{
  m_gains = gains;
  if (m_loaded) {
    applyGains();
  }
}

void MpvItem::setAudioDelay(double seconds)
{
  if (m_mpv) {
    mpv_set_property_string(m_mpv, "audio-delay", QByteArray::number(qBound(-5.0, seconds, 5.0), 'f', 3).constData());
  }
}

bool MpvItem::saveFrame(const QString& path)
{
  if (!m_mpv || !m_loaded) {
    return false;
  }
  const QByteArray file = path.toUtf8();
  // The picture alone, as it was recorded: not the window's size.
  const char* args[] = { "screenshot-to-file", file.constData(), "video", nullptr };
  return mpv_command(m_mpv, args) >= 0;
}

// The tracks, each at its own volume, mixed into one. The mix is built
// once for a file; after that a volume is changed inside it, which is
// heard at once and without a gap. (Building it again restarts the sound,
// which is a stutter while a slider is dragged.)
void MpvItem::applyGains()
{
  if (!m_mpv) {
    return;
  }
  if (m_gains.size() != m_mixed) {
    buildMix();
    return;
  }
  for (int i = 0; i < m_gains.size(); ++i) {
    const double gain = qBound(0.0, m_gains[i].toDouble(), 2.0);
    if (qFuzzyCompare(gain + 1, m_heard[i] + 1)) {
      continue;
    }
    m_heard[i] = gain;
    command({ "af-command", "gains", "volume", QString::number(gain, 'f', 3), QStringLiteral("volume@t%1").arg(i) });
  }
}

// Every track is brought to stereo and laid side by side (track 1 in the
// first two channels, track 2 in the next two); a filter after that takes
// each pair out, sets its volume (`volume@t0`, `volume@t1`, which can be
// told a new volume while playing) and adds them up.
void MpvItem::buildMix()
{
  const int tracks = m_gains.size();
  m_mixed = tracks;
  m_heard.clear();
  for (const QVariant& gain : m_gains) {
    m_heard.append(qBound(0.0, gain.toDouble(), 2.0));
  }
  QString merge;
  QString mix;
  if (tracks == 1) {
    mix = QStringLiteral("volume@t0=%1").arg(m_heard[0], 0, 'f', 3);
  } else if (tracks > 1) {
    QString pairs;
    QString sums;
    mix = QStringLiteral("asplit=%1").arg(tracks);
    for (int i = 0; i < tracks; ++i) {
      merge += QStringLiteral("[aid%1]aresample=48000,aformat=channel_layouts=stereo[a%1];").arg(i + 1);
      pairs += QStringLiteral("[a%1]").arg(i + 1);
      mix += QStringLiteral("[s%1]").arg(i);
      sums += QStringLiteral("[m%1]").arg(i);
    }
    merge += pairs + QStringLiteral("amerge=inputs=%1[ao]").arg(tracks);
    for (int i = 0; i < tracks; ++i) {
      mix += QStringLiteral(";[s%1]pan=stereo|c0=c%2|c1=c%3,volume@t%1=%4[m%1]").arg(i).arg(2 * i).arg(2 * i + 1).arg(m_heard[i], 0, 'f', 3);
    }
    mix += QLatin1Char(';') + sums + QStringLiteral("amix=inputs=%1:normalize=0").arg(tracks);
  }
  // The filter of a mix before this one must not meet the new channels.
  mpv_set_property_string(m_mpv, "af", "");
  mpv_set_property_string(m_mpv, "lavfi-complex", merge.toUtf8().constData());
  if (merge.isEmpty()) {
    mpv_set_property_string(m_mpv, "aid", tracks == 1 ? "1" : "no");
  }
  if (!mix.isEmpty()) {
    mpv_set_property_string(m_mpv, "af", (QStringLiteral("@gains:lavfi=[") + mix + QLatin1Char(']')).toUtf8().constData());
  }
}

void MpvItem::drainEvents()
{
  while (m_mpv) {
    mpv_event* event = mpv_wait_event(m_mpv, 0);
    if (event->event_id == MPV_EVENT_NONE) {
      break;
    }
    switch (event->event_id) {
      case MPV_EVENT_FILE_LOADED:
        m_loaded = true;
        emit loadedChanged();
        if (!m_gains.isEmpty()) {
          applyGains();
        }
        break;
      case MPV_EVENT_END_FILE: {
        auto* end = static_cast<mpv_event_end_file*>(event->data);
        if (end->reason == MPV_END_FILE_REASON_ERROR) {
          emit failed(QString::fromUtf8(mpv_error_string(end->error)));
        }
        break;
      }
      case MPV_EVENT_PROPERTY_CHANGE: {
        auto* prop = static_cast<mpv_event_property*>(event->data);
        const QByteArray name(prop->name);
        if (prop->format == MPV_FORMAT_DOUBLE) {
          const double value = *static_cast<double*>(prop->data);
          if (name == "time-pos") {
            m_position = value;
            emit positionChanged();
          } else if (name == "duration") {
            m_duration = value;
            emit durationChanged();
          }
        } else if (prop->format == MPV_FORMAT_FLAG) {
          const bool value = *static_cast<int*>(prop->data) != 0;
          if (name == "pause" && value != m_paused) {
            m_paused = value;
            emit pausedChanged();
          } else if (name == "eof-reached" && value) {
            emit ended();
          }
        }
        break;
      }
      default:
        break;
    }
  }
}

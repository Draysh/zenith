// The video in the editor: libmpv drawing into a Qt Quick item, with
// hardware decoding and seeking to the exact frame. Registered in QML as
// `MpvPlayer`.

#pragma once

#include <QtCore/QString>
#include <QtCore/QVariantList>
#include <QtQml/qqmlregistration.h>
#include <QtQuick/QQuickFramebufferObject>

#include <memory>

struct mpv_handle;
struct mpv_render_context;
struct MpvShared;

class MpvItem : public QQuickFramebufferObject
{
  Q_OBJECT
  QML_NAMED_ELEMENT(MpvPlayer)
  Q_PROPERTY(QString source READ source WRITE setSource NOTIFY sourceChanged)
  // Where playback is and how long the file is, in seconds.
  Q_PROPERTY(double position READ position NOTIFY positionChanged)
  Q_PROPERTY(double duration READ duration NOTIFY durationChanged)
  Q_PROPERTY(bool paused READ paused WRITE setPaused NOTIFY pausedChanged)
  // 0 to 1.
  Q_PROPERTY(double volume READ volume WRITE setVolume NOTIFY volumeChanged)
  // A file is loaded and can be played and seeked.
  Q_PROPERTY(bool loaded READ loaded NOTIFY loadedChanged)
  // The picture can be drawn: mpv has its OpenGL drawing context. Where
  // this never comes true, there is no picture to show.
  Q_PROPERTY(bool canDraw READ canDraw NOTIFY canDrawChanged)

public:
  explicit MpvItem(QQuickItem* parent = nullptr);
  ~MpvItem() override;
  Renderer* createRenderer() const override;

  QString source() const { return m_source; }
  void setSource(const QString& path);
  double position() const { return m_position; }
  double duration() const { return m_duration; }
  bool paused() const { return m_paused; }
  void setPaused(bool paused);
  double volume() const { return m_volume; }
  void setVolume(double volume);
  bool loaded() const { return m_loaded; }
  bool canDraw() const { return m_canDraw; }

  // Go to `seconds`. Exact lands on that frame; otherwise on the keyframe
  // before it, which is fast enough to follow a drag.
  Q_INVOKABLE void seek(double seconds, bool exact = true);
  // One frame forward (1) or back (-1), paused.
  Q_INVOKABLE void step(int frames);
  // How loud each audio track of the file is (1 is as recorded, 0 silent),
  // mixed into what is heard.
  Q_INVOKABLE void setTrackGains(const QVariantList& gains);
  // Play the sound this much later (or, below zero, earlier) against the
  // picture, in seconds.
  Q_INVOKABLE void setAudioDelay(double seconds);
  // Save the frame that is showing, at the clip's own size, as a picture
  // at `path` (its ending says of which kind). False when it could not be.
  Q_INVOKABLE bool saveFrame(const QString& path);


signals:
  void sourceChanged();
  void positionChanged();
  void durationChanged();
  void pausedChanged();
  void volumeChanged();
  void loadedChanged();
  void canDrawChanged();
  // Playback reached the end of the file.
  void ended();
  // The file could not be played.
  void failed(const QString& reason);

private slots:
  void drainEvents();
  void renderReady();
  void renderFailed();

private:
  void command(const QStringList& args);
  void applyGains();
  void buildMix();
  void load();

  mpv_handle* m_mpv = nullptr;
  // What the renderer draws with. It lives on the render thread and can
  // outlive this item, so the two share this instead of each other.
  std::shared_ptr<MpvShared> m_shared;
  QString m_source;
  double m_position = 0;
  double m_duration = 0;
  double m_volume = 1;
  bool m_paused = true;
  bool m_loaded = false;
  bool m_canDraw = false;
  QVariantList m_gains;
  // How many tracks the mix in place is for (-1: none built yet), and the
  // volume each was last told.
  int m_mixed = -1;
  QList<double> m_heard;
};

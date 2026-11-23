//! What the recorder can do on this machine, asked of the recorder itself:
//! the monitors (their names and sizes) and the codecs the GPU encodes.


#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Output {
    pub name: String,
    pub size: (u32, u32),
}

/// Read the recorder's `--list-capture-options`: a line a monitor,
/// `DP-1|2560x1440`, among other things it can capture (`region`, `portal`).
pub fn parse(text: &str) -> Vec<Output> {
    text.lines()
        .filter_map(|line| {
            let (name, size) = line.trim().split_once('|')?;
            let (w, h) = size.split_once('x')?;
            let size = (w.parse().ok()?, h.parse().ok()?);
            (size.0 > 0 && size.1 > 0).then(|| Output { name: name.to_string(), size })
        })
        .collect()
}

/// The monitors the recorder can capture, as it sees them (a turned monitor
/// has its sides swapped). Empty when it cannot be asked.
pub fn list(recorder: &str) -> Vec<Output> {
    crate::paths::command(recorder)
        .arg("--list-capture-options")
        .output()
        .map(|out| parse(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// Read the codecs out of the recorder's `--info`: the lines of its
/// `video_codecs` section.
pub fn parse_codecs(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .skip_while(|l| *l != "section=video_codecs")
        .skip(1)
        .take_while(|l| !l.starts_with("section="))
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// The codecs this machine's GPU can record with (`h264`, `hevc`, `av1`
/// among others). Empty when the recorder cannot be asked.
pub fn codecs(recorder: &str) -> Vec<String> {
    crate::paths::command(recorder)
        .arg("--info")
        .output()
        .map(|out| parse_codecs(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// What to record for a `source` setting: the recorder's `-w` value and the
/// size of the picture. `screen` is the largest monitor, the first of them
/// when several are as large; a monitor's name is that monitor. When the recorder cannot be asked, the setting is passed
/// on as it is and 1080p is assumed.
pub fn resolve(source: &str, outputs: &[Output]) -> (String, (u32, u32)) {
    let area = |o: &&Output| o.size.0 as u64 * o.size.1 as u64;
    // `max_by_key` keeps the last of equals; looked through backwards, that is the first.
    let largest = outputs.iter().rev().max_by_key(area);
    let picked = match source {
        "screen" => largest.filter(|o| !o.name.is_empty()),
        "portal" => None,
        name => outputs.iter().find(|o| o.name == name),
    };
    match picked {
        Some(o) => (o.name.clone(), o.size),
        None => (source.to_string(), largest.map_or((1920, 1080), |o| o.size)),
    }
}

/// Something sound can be recorded from: what plays on a set of speakers
/// or headphones, or a microphone.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioDevice {
    /// The name the recorder knows it by.
    pub name: String,
    /// The name a person knows it by.
    pub label: String,
}

/// What stands for "whichever is the default" among outputs and inputs.
pub const DEFAULT_OUTPUT: &str = "default_output";
pub const DEFAULT_INPUT: &str = "default_input";

impl AudioDevice {
    /// A microphone, as opposed to what is playing on an output. The
    /// recorder lists an output as the "monitor" of it.
    pub fn is_input(&self) -> bool {
        self.name != DEFAULT_OUTPUT && !self.name.ends_with(".monitor")
    }

    /// The label without the recorder's "Monitor of " in front.
    pub fn plain_label(&self) -> &str {
        self.label.strip_prefix("Monitor of ").unwrap_or(&self.label)
    }
}

/// The recorder's `--list-audio-devices`: a `name|description` on each line.
pub fn parse_audio_devices(text: &str) -> Vec<AudioDevice> {
    text.lines()
        .filter_map(|line| {
            let (name, label) = line.trim().split_once('|')?;
            (!name.is_empty() && !name.contains(' ')).then(|| AudioDevice { name: name.into(), label: label.trim().into() })
        })
        .collect()
}

/// The devices sound can be recorded from here; none when the recorder
/// cannot be asked.
pub fn audio_devices(recorder: &str) -> Vec<AudioDevice> {
    let out = crate::paths::command(recorder).arg("--list-audio-devices").stdin(std::process::Stdio::null()).output();
    out.ok().filter(|o| o.status.success()).map(|o| parse_audio_devices(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// The device to record from: the one chosen while it is plugged in, the
/// default one when it is not. With no list to go by, the choice stands.
pub fn present(chosen: &str, default: &str, devices: &[AudioDevice]) -> String {
    if devices.is_empty() || devices.iter().any(|d| d.name == chosen) { chosen.to_string() } else { default.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_devices_are_read_and_told_apart() {
        let devices = parse_audio_devices(
            "default_output|Default output\ndefault_input|Default input\nalsa_output.usb-FiiO.analog-stereo.monitor|Monitor of FiiO K7 Analog Stereo\nalsa_input.usb-Wave_3.mono-fallback|Elgato Wave 3 Mono\nWARNING: no line like the others\n",
        );
        assert_eq!(devices.len(), 4);
        let inputs: Vec<&str> = devices.iter().filter(|d| d.is_input()).map(|d| d.name.as_str()).collect();
        assert_eq!(inputs, ["default_input", "alsa_input.usb-Wave_3.mono-fallback"]);
        assert_eq!(devices[2].plain_label(), "FiiO K7 Analog Stereo");
        assert_eq!(devices[3].plain_label(), "Elgato Wave 3 Mono");
    }

    #[test]
    fn a_device_that_was_unplugged_gives_way_to_the_default() {
        let devices = parse_audio_devices("default_input|Default input\nmic.usb|USB microphone\n");
        assert_eq!(present("mic.usb", DEFAULT_INPUT, &devices), "mic.usb");
        assert_eq!(present("mic.gone", DEFAULT_INPUT, &devices), DEFAULT_INPUT);
        assert_eq!(present("mic.gone", DEFAULT_INPUT, &[]), "mic.gone", "with nothing to go by, the choice stands");
    }

    fn out(name: &str, w: u32, h: u32) -> Output {
        Output { name: name.into(), size: (w, h) }
    }

    #[test]
    fn the_recorder_list_is_read() {
        let outs = parse("DP-1|2560x1440\nHDMI-A-1|1080x1920\nregion\nportal\n");
        assert_eq!(outs, [out("DP-1", 2560, 1440), out("HDMI-A-1", 1080, 1920)]);
        assert!(parse("WARNING: something\n").is_empty());
    }

    #[test]
    fn the_codec_list_is_read() {
        let info = "section=system_info\ndisplay_server|wayland\nsection=video_codecs\nh264\nhevc\nav1\nsection=image_formats\njpeg\n";
        assert_eq!(parse_codecs(info), ["h264", "hevc", "av1"]);
        assert!(parse_codecs("nothing").is_empty());
    }

    #[test]
    fn the_source_setting_picks_a_monitor() {
        let outs = [out("HDMI-A-1", 1920, 1080), out("DP-1", 2560, 1440)];
        assert_eq!(resolve("screen", &outs), ("DP-1".into(), (2560, 1440)));
        assert_eq!(resolve("HDMI-A-1", &outs), ("HDMI-A-1".into(), (1920, 1080)));
        assert_eq!(resolve("DP-9", &outs), ("DP-9".into(), (2560, 1440)), "an unknown name is passed on");
        assert_eq!(resolve("portal", &outs), ("portal".into(), (2560, 1440)));
        assert_eq!(resolve("screen", &[]), ("screen".into(), (1920, 1080)));
    }

    #[test]
    fn of_monitors_as_large_the_first_is_recorded() {
        let outs = [out("DP-1", 1920, 1080), out("HDMI-A-1", 1920, 1080), out("DP-2", 1080, 1920)];
        assert_eq!(resolve("screen", &outs), ("DP-1".into(), (1920, 1080)));
    }
}

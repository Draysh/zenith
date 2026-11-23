//! `zenith-rec`: the replay-buffer recorder Zenith starts on Windows, where
//! `gpu-screen-recorder` does not exist. Everything it does is in the library.

fn main() -> std::process::ExitCode {
    zenith_rec::run(std::env::args().skip(1).collect())
}

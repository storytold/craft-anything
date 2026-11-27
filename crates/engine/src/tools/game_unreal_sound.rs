//! Taking a sound out of an Unreal game: a cooked SoundWave saved as the WAV or Ogg file the game
//! stores, without re-encoding.

use super::game_archive_tools::{UnrealExport, new_output, unpack_permission, unreal_export, with_sound_extension};
use super::native_common::{params_json, spec};
use super::static_input::{builder, format_err};
use crate::tool::{Ctx, Effects, Requires, ToolOutput, ToolSpec, Toolset, parse_params, schema_of};
use craft_anything_evidence::{Confidence, Coverage, Location, Subject, ToolError};
use craft_anything_formats::unreal_sound::{self, Place};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        "game_unreal_sound_export",
        "Take a sound out of an Unreal game",
        "Save a sound (SoundWave) of an Unreal Engine 4.26-4.27 game, only when the person confirms they may unpack the game (permission: true; they made it or its rights holder allows it). path is the game's .pak (then `file` is the sound's .uasset path inside it, as game_pak_list shows it) or a .uasset already taken out, with its .uexp (and .ubulk) beside it. The sound is saved as the game stores it, without re-encoding: a WAV file (PCM, ADPCM…) or an Ogg file (Vorbis, Opus), at `output` (an absolute path that doesn't exist yet) with its extension set to .wav or .ogg to match; the result names the codec, channels and rate. Sounds stored any other way (Bink Audio, a game's own codec, streamed in parts) are refused, as are Oodle-compressed and encrypted paks, Unreal 5 packages and packages written without property names.",
        "Save a sound from an Unreal game you may unpack, as a WAV or Ogg file.",
        Toolset::Formats,
        Effects::EVIDENCE.union(Effects::WRITES_FILESYSTEM),
        Requires::Nothing,
        schema_of::<UnrealSoundParams>,
        game_unreal_sound_export,
    )]
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnrealSoundParams {
    /// Absolute path of the game's .pak, or of a .uasset already taken out (its .uexp and .ubulk beside it).
    pub path: String,
    /// For a pak: the sound's .uasset path inside it, as game_pak_list shows it.
    #[serde(default)]
    pub file: Option<String>,
    /// Where to write the sound: an absolute path that doesn't exist yet (its folders are made); its extension becomes .wav or .ogg.
    pub output: String,
    /// The person made this game or has its rights holder's permission to unpack its files.
    #[serde(default)]
    pub permission: bool,
}

fn game_unreal_sound_export(ctx: &mut Ctx<'_>, params: Value) -> Result<ToolOutput, ToolError> {
    let p: UnrealSoundParams = parse_params(params)?;
    unpack_permission(p.permission)?;
    new_output(&p.output)?;
    let UnrealExport { mut source, shown_name, names, export, data } = unreal_export(ctx, &p.path, p.file.as_deref(), "SoundWave", "sound")?;
    let cancel = ctx.cancel.clone();
    let ubulk = source.part("ubulk", &cancel)?;
    let sound = unreal_sound::sound(&data, &names, ubulk.as_deref()).map_err(|e| format_err(e, "an Unreal sound"))?;
    let file = match sound.place {
        Place::Uexp => data.get(sound.range.0..sound.range.0 + sound.range.1),
        Place::Ubulk => ubulk.as_deref().and_then(|u| u.get(sound.range.0..sound.range.0 + sound.range.1)),
    }
    .ok_or_else(|| ToolError::internal("a found sound lies outside its file"))?;
    // What's stored isn't known before the sound is read, so the file gets the extension it needs.
    let out = with_sound_extension(std::path::Path::new(&p.output), sound.container.extension());
    if out.exists() {
        return Err(ToolError::invalid(format!("{} already exists", out.display())).with_remediation("Pick a new file name; nothing is overwritten."));
    }
    crate::project::write_atomic(&out, file)?;
    let result = json!({
        "name": export.name,
        "class": "SoundWave",
        "saved_as": sound.container,
        "codec": sound.codec,
        "channels": sound.channels,
        "rate": sound.rate,
        "from": sound.place,
        "output": out.display().to_string(),
        "size": file.len(),
        "sha256": craft_anything_evidence::sha256_hex(file),
    });
    let b = builder(ctx, "game_unreal_sound_export", Subject { kind: "archive".into(), sha256: None, name: Some(shown_name) }, params_json(&p))
        .result(result)
        .summary(format!("Saved sound \"{}\" ({}, {} channel(s), {} Hz) as {}.", export.name, sound.codec, sound.channels, sound.rate, out.display()))
        .confidence(Confidence::Observed)
        .coverage(Coverage::complete(1))
        .location(Location::new("path", out.display().to_string()));
    Ok(ToolOutput::evidence(b.build()?))
}

#[cfg(test)]
mod tests {
    use crate::{CallOptions, Caller, Engine};
    use craft_anything_testkit::games::{ogg_file, unreal_pak, unreal_sound_package, wav_file};
    use serde_json::json;

    #[test]
    fn saves_unreal_sounds_as_they_are_stored() {
        let e = Engine::new(crate::testing::session());
        let dir = tempfile::tempdir().unwrap();
        let wav = wav_file(0x11, 1, 22050, &[3; 40]);
        let (click_uasset, click_uexp) = unreal_sound_package("S_Click", "ADPCM", &wav, false);
        let ogg = ogg_file(b"\x01vorbis", 2, 44100, 3);
        let (theme_uasset, theme_uexp) = unreal_sound_package("S_Theme", "OGG", &ogg, true);
        let pak = unreal_pak(&[
            ("Game/Audio/", "S_Click.uasset", &click_uasset, false),
            ("Game/Audio/", "S_Click.uexp", &click_uexp, true),
            ("Game/Audio/", "S_Theme.uasset", &theme_uasset, false),
            ("Game/Audio/", "S_Theme.uexp", &theme_uexp, false),
            ("Game/Audio/", "S_Theme.ubulk", &ogg, false),
        ]);
        let pak_path = dir.path().join("Game-Windows.pak");
        std::fs::write(&pak_path, pak).unwrap();
        let call = |params: serde_json::Value| e.call("game_unreal_sound_export", params, CallOptions::new(Caller::Mcp));
        let out = |name: &str| dir.path().join(name);

        let err = call(json!({"path": pak_path, "file": "Game/Audio/S_Click.uasset", "output": out("click")})).unwrap_err();
        assert_eq!(err.code, craft_anything_evidence::ErrorCode::ConsentRequired, "{err:?}");

        let v = call(json!({"path": pak_path, "file": "Game/Audio/S_Click.uasset", "output": out("click"), "permission": true})).unwrap().to_json();
        assert_eq!((v["result"]["saved_as"].as_str(), v["result"]["codec"].as_str()), (Some("wav"), Some("IMA ADPCM")), "{v}");
        assert_eq!(std::fs::read(out("click.wav")).unwrap(), wav, "the stored WAV, unchanged");

        // A wrong extension is swapped for the one the stored file needs.
        let v = call(json!({"path": pak_path, "file": "game/audio/s_theme.uasset", "output": out("theme.wav"), "permission": true})).unwrap().to_json();
        assert_eq!((v["result"]["from"].as_str(), v["result"]["channels"].as_u64()), (Some("ubulk"), Some(2)), "{v}");
        assert_eq!(std::fs::read(out("theme.ogg")).unwrap(), ogg);
        assert!(!out("theme.wav").exists());

        // Loose files; a texture package isn't a sound.
        std::fs::write(out("S_Click.uasset"), &click_uasset).unwrap();
        std::fs::write(out("S_Click.uexp"), &click_uexp).unwrap();
        let v = call(json!({"path": out("S_Click.uasset"), "output": out("loose.wav"), "permission": true})).unwrap().to_json();
        assert_eq!(v["result"]["name"], "S_Click");
        let (t_uasset, t_uexp) = craft_anything_testkit::games::unreal_texture_package("T", "PF_DXT1", 4, 4, &[0; 8], None);
        std::fs::write(out("T.uasset"), &t_uasset).unwrap();
        std::fs::write(out("T.uexp"), &t_uexp).unwrap();
        let err = call(json!({"path": out("T.uasset"), "output": out("t.wav"), "permission": true})).unwrap_err();
        assert!(err.message.contains("isn't a sound"), "{err:?}");
    }
}

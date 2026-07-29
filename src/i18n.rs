//! Compile-time translation table (English / German / French).
//!
//! Every user-visible string lives here behind a stable key. `t` returns the
//! literal for the active language; `tf` fills `{}` placeholders in order.
//! Unknown keys fall back to the key itself so a typo is visible in the UI
//! instead of silently rendering an empty label.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    De,
    Fr,
}

/// Selectable languages, in the order the picker shows them.
pub const LANGS: [Lang; 3] = [Lang::En, Lang::De, Lang::Fr];

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Fr => "fr",
        }
    }

    /// Anything unrecognised falls back to English.
    pub fn from_code(code: &str) -> Lang {
        match code {
            "de" => Lang::De,
            "fr" => Lang::Fr,
            _ => Lang::En,
        }
    }

    /// Endonym - a language is easiest to find written in itself.
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
        }
    }
}

/// Keys are always literals, so an unknown one can be handed straight back
/// without allocating - a missing translation shows up as its own key.
pub fn t(lang: Lang, key: &'static str) -> &'static str {
    let (en, de, fr) = entry(key);
    match lang {
        Lang::En => en,
        Lang::De => de,
        Lang::Fr => fr,
    }
}

/// Substitutes `args` into the `{}` placeholders of `key`'s translation.
///
/// Substitution never re-scans inserted text, so an argument that itself
/// contains `{}` (a file name, say) cannot swallow the next placeholder.
pub fn tf(lang: Lang, key: &'static str, args: &[&str]) -> String {
    let template = t(lang, key);
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    let mut next = 0;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        match args.get(next) {
            Some(a) => out.push_str(a),
            None => out.push_str("{}"),
        }
        next += 1;
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

#[rustfmt::skip]
fn entry(key: &'static str) -> (&'static str, &'static str, &'static str) {
    match key {
        // ---------------- shared ----------------
        "common.language" => ("Language", "Sprache", "Langue"),
        "common.on" => ("on", "an", "activé"),
        "common.off" => ("off", "aus", "désactivé"),
        "common.choose" => ("Choose", "Auswählen", "Parcourir"),
        "common.clear" => ("Clear", "Leeren", "Effacer"),
        "common.stop" => ("⏹ Stop", "⏹ Stopp", "⏹ Arrêter"),
        "common.live_log" => ("Live Log", "Live-Log", "Journal en direct"),
        "common.error" => ("Error: {}", "Fehler: {}", "Erreur : {}"),

        // ---------------- navigation ----------------
        "nav.download" => ("Download", "Download", "Téléchargement"),
        "nav.convert" => ("Convert", "Konvertieren", "Convertir"),
        "nav.setup" => ("Setup", "Einrichtung", "Installation"),
        "nav.info" => ("Info", "Info", "Infos"),

        // ---------------- footer ----------------
        "footer.hw" => ("HW: {}", "HW: {}", "Matériel : {}"),
        "footer.impersonate" => ("Impersonate: {}", "Impersonate: {}", "Impersonate : {}"),
        "footer.js" => ("JS: {}", "JS: {}", "JS : {}"),

        // ---------------- download tab ----------------
        "dl.subtitle" => (
            "Videos & audio from YouTube, Instagram, TikTok, X and 1700+ sites",
            "Videos & Audio von YouTube, Instagram, TikTok, X und über 1700 weiteren Seiten",
            "Vidéos et audio depuis YouTube, Instagram, TikTok, X et plus de 1700 sites",
        ),
        "dl.source_quality" => ("Source & Quality", "Quelle & Qualität", "Source et qualité"),
        "dl.url" => ("URL:", "URL:", "URL :"),
        "dl.url_hint" => (
            "YouTube, TikTok, X/Twitter, Instagram ...",
            "YouTube, TikTok, X/Twitter, Instagram ...",
            "YouTube, TikTok, X/Twitter, Instagram ...",
        ),
        "dl.paste" => ("📋 Paste", "📋 Einfügen", "📋 Coller"),
        "dl.quality" => ("Quality", "Qualität", "Qualité"),
        "dl.cookies" => ("Cookies", "Cookies", "Cookies"),
        "dl.conflict" => ("If file exists", "Wenn die Datei existiert", "Si le fichier existe"),
        "dl.cookiefile" => ("cookies.txt:", "cookies.txt:", "cookies.txt :"),
        "dl.advanced" => ("Advanced options", "Erweiterte Optionen", "Options avancées"),
        "dl.impersonate" => ("Impersonate (Anti-Bot)", "Impersonate (Anti-Bot)", "Impersonate (anti-bot)"),
        "dl.sponsorblock" => (
            "SponsorBlock (Remove Sponsors)",
            "SponsorBlock (Sponsoren entfernen)",
            "SponsorBlock (supprimer les sponsors)",
        ),
        "dl.embed" => (
            "Embed Thumbnail/Metadata/Chapters",
            "Thumbnail/Metadaten/Kapitel einbetten",
            "Intégrer miniature/métadonnées/chapitres",
        ),
        "dl.subs" => ("Download Subtitles", "Untertitel herunterladen", "Télécharger les sous-titres"),
        "dl.subs_langs" => ("Languages:", "Sprachen:", "Langues :"),
        "dl.potoken" => (
            "PO Token / mweb (for 18+ Videos)",
            "PO Token / mweb (für 18+ Videos)",
            "Jeton PO / mweb (vidéos 18+)",
        ),
        "dl.provider_url" => ("Provider URL:", "Provider-URL:", "URL du fournisseur :"),
        "dl.provider_hint" => (
            "empty = http://127.0.0.1:4416",
            "leer = http://127.0.0.1:4416",
            "vide = http://127.0.0.1:4416",
        ),
        "dl.save_location" => ("Save Location", "Speicherort", "Emplacement d'enregistrement"),
        "dl.choose_folder" => ("📁 Choose folder", "📁 Ordner wählen", "📁 Choisir un dossier"),
        "dl.choose_folder_hint" => (
            "Pick the folder downloads are saved to",
            "Ordner wählen, in den Downloads gespeichert werden",
            "Choisir le dossier où enregistrer les téléchargements",
        ),
        "dl.choose_folder_title" => (
            "Choose save folder",
            "Zielordner wählen",
            "Choisir le dossier de destination",
        ),
        "dl.open_folder" => ("↗ Open", "↗ Öffnen", "↗ Ouvrir"),
        "dl.open_folder_hint" => (
            "Open this folder in the file manager",
            "Diesen Ordner im Datei-Explorer öffnen",
            "Ouvrir ce dossier dans l'explorateur de fichiers",
        ),
        "dl.start" => ("⬇ Start Download", "⬇ Download starten", "⬇ Démarrer le téléchargement"),

        // ---------------- statuses ----------------
        "status.ready" => ("Ready", "Bereit", "Prêt"),
        "status.downloading" => ("Downloading ...", "Lade herunter ...", "Téléchargement ..."),
        "status.converting" => ("Converting ...", "Konvertiere ...", "Conversion ..."),
        "status.cancelled" => ("Cancelled", "Abgebrochen", "Annulé"),
        "log.ready_downloads" => (
            "  Ready for downloads ...",
            "  Bereit für Downloads ...",
            "  Prêt pour les téléchargements ...",
        ),
        "log.ready_conversion" => (
            "  Ready for conversion ...",
            "  Bereit zum Konvertieren ...",
            "  Prêt pour la conversion ...",
        ),

        // ---------------- toasts ----------------
        "toast.need_url" => (
            "Please enter a URL first.",
            "Bitte zuerst eine URL eingeben.",
            "Veuillez d'abord saisir une URL.",
        ),
        "toast.bad_url" => (
            "URL must start with http:// or https://.",
            "Die URL muss mit http:// oder https:// beginnen.",
            "L'URL doit commencer par http:// ou https://.",
        ),
        "toast.need_folder" => (
            "Please choose a save folder.",
            "Bitte einen Zielordner wählen.",
            "Veuillez choisir un dossier de destination.",
        ),
        "toast.no_ytdlp" => (
            "yt-dlp not available - please run Setup.",
            "yt-dlp nicht verfügbar - bitte die Einrichtung ausführen.",
            "yt-dlp indisponible - veuillez lancer l'installation.",
        ),
        "toast.no_cookiefile" => (
            "Cookies file (cookies.txt) is missing or not found.",
            "Cookies-Datei (cookies.txt) fehlt oder wurde nicht gefunden.",
            "Fichier de cookies (cookies.txt) manquant ou introuvable.",
        ),
        "toast.mkdir_failed" => (
            "Could not create folder: {}",
            "Ordner konnte nicht erstellt werden: {}",
            "Impossible de créer le dossier : {}",
        ),
        "toast.need_io" => (
            "Please choose input and output files.",
            "Bitte Eingabe- und Ausgabedatei wählen.",
            "Veuillez choisir les fichiers d'entrée et de sortie.",
        ),
        "toast.no_input" => (
            "Input file does not exist.",
            "Die Eingabedatei existiert nicht.",
            "Le fichier d'entrée n'existe pas.",
        ),
        "toast.same_io" => (
            "Input and output must not be identical.",
            "Eingabe und Ausgabe dürfen nicht identisch sein.",
            "L'entrée et la sortie ne doivent pas être identiques.",
        ),
        "toast.no_ffmpeg" => (
            "FFmpeg not available - please run Setup.",
            "FFmpeg nicht verfügbar - bitte die Einrichtung ausführen.",
            "FFmpeg indisponible - veuillez lancer l'installation.",
        ),
        "toast.mkdir_out_failed" => (
            "Could not create target folder: {}",
            "Zielordner konnte nicht erstellt werden: {}",
            "Impossible de créer le dossier de destination : {}",
        ),

        // ---------------- convert tab ----------------
        "cv.subtitle" => (
            "FFmpeg conversion with live progress",
            "FFmpeg-Konvertierung mit Live-Fortschritt",
            "Conversion FFmpeg avec progression en direct",
        ),
        "cv.input" => ("Input:", "Eingabe:", "Entrée :"),
        "cv.browse" => ("📂 Browse…", "📂 Durchsuchen…", "📂 Parcourir…"),
        "cv.output" => ("Output:", "Ausgabe:", "Sortie :"),
        "cv.save_as" => ("💾 Save as…", "💾 Speichern unter…", "💾 Enregistrer sous…"),
        "cv.filter_media" => ("Video/Audio", "Video/Audio", "Vidéo/Audio"),
        "cv.filter_all" => ("All files", "Alle Dateien", "Tous les fichiers"),
        "cv.category" => ("Category", "Kategorie", "Catégorie"),
        "cv.codec" => ("Codec", "Codec", "Codec"),
        "cv.hardware" => ("Hardware", "Hardware", "Matériel"),
        "cv.bitrate_mode" => ("Bitrate Mode", "Bitraten-Modus", "Mode de débit"),
        "cv.crf" => ("CRF", "CRF", "CRF"),
        "cv.mbps" => ("Mbps", "Mbit/s", "Mbit/s"),
        "cv.preserve_color" => (
            "Preserve Color Metadata (BT.709 / BT.2020)",
            "Farb-Metadaten erhalten (BT.709 / BT.2020)",
            "Conserver les métadonnées de couleur (BT.709 / BT.2020)",
        ),
        "cv.start" => ("▶ Convert", "▶ Konvertieren", "▶ Convertir"),
        "cv.clear_log" => ("Clear Log", "Log leeren", "Effacer le journal"),
        "cv.hint_audio_only" => (
            "Note: audio only - the video track is discarded.",
            "Hinweis: nur Audio - die Videospur wird verworfen.",
            "Remarque : audio seul - la piste vidéo est ignorée.",
        ),
        "cv.hint_cpu" => (
            "Note: this codec runs most stably on CPU.",
            "Hinweis: Dieser Codec läuft auf der CPU am stabilsten.",
            "Remarque : ce codec est le plus stable sur le processeur.",
        ),
        "cv.hint_copy" => (
            "Note: stream copy - no re-encoding.",
            "Hinweis: Stream-Kopie - keine Neukodierung.",
            "Remarque : copie du flux - pas de réencodage.",
        ),
        "cv.hint_av1" => (
            "Note: AV1 - very efficient, but slow on CPU.",
            "Hinweis: AV1 - sehr effizient, aber langsam auf der CPU.",
            "Remarque : AV1 - très efficace, mais lent sur le processeur.",
        ),

        // ---------------- setup tab ----------------
        "setup.subtitle" => (
            "Manage yt-dlp and FFmpeg",
            "yt-dlp und FFmpeg verwalten",
            "Gérer yt-dlp et FFmpeg",
        ),
        "setup.deno" => ("Deno (JS Runtime)", "Deno (JS-Laufzeit)", "Deno (moteur JS)"),
        "setup.not_installed" => ("Not installed", "Nicht installiert", "Non installé"),
        "setup.bin_path" => ("Local Bin Path:", "Lokaler Bin-Pfad:", "Chemin bin local :"),
        "setup.install" => ("⬇ Install Binaries", "⬇ Binaries installieren", "⬇ Installer les binaires"),
        "setup.channel" => ("Channel:", "Kanal:", "Canal :"),
        "setup.update_ytdlp" => ("⟳ Update yt-dlp", "⟳ yt-dlp aktualisieren", "⟳ Mettre à jour yt-dlp"),
        "setup.install_deno" => ("Install Deno", "Deno installieren", "Installer Deno"),
        "setup.busy_hint" => (
            "An installation is already running - please wait.",
            "Eine Installation läuft bereits - bitte warten.",
            "Une installation est déjà en cours - veuillez patienter.",
        ),
        "setup.installing" => (
            "Installing binaries ...",
            "Installiere Binaries ...",
            "Installation des binaires ...",
        ),
        "setup.dl_ytdlp" => ("Downloading yt-dlp", "Lade yt-dlp herunter", "Téléchargement de yt-dlp"),
        "setup.dl_ffmpeg" => (
            "Downloading FFmpeg ...",
            "Lade FFmpeg herunter ...",
            "Téléchargement de FFmpeg ...",
        ),
        "setup.dl_named" => ("Downloading {}", "Lade {} herunter", "Téléchargement de {}"),
        "setup.install_done" => (
            "Installation completed",
            "Installation abgeschlossen",
            "Installation terminée",
        ),
        "setup.install_error" => (
            "Installation error: {}",
            "Installationsfehler: {}",
            "Erreur d'installation : {}",
        ),
        "setup.install_ok_toast" => (
            "Binaries installed successfully",
            "Binaries erfolgreich installiert",
            "Binaires installés avec succès",
        ),
        "setup.updating_ytdlp" => ("Updating yt-dlp", "Aktualisiere yt-dlp", "Mise à jour de yt-dlp"),
        "setup.updating_ytdlp_status" => (
            "Updating yt-dlp ...",
            "Aktualisiere yt-dlp ...",
            "Mise à jour de yt-dlp ...",
        ),
        "setup.switch_channel" => (
            "Switching yt-dlp to the '{}' channel ...",
            "Wechsle yt-dlp auf den Kanal „{}“ ...",
            "Passage de yt-dlp au canal « {} » ...",
        ),
        "setup.ytdlp_updated" => ("yt-dlp updated", "yt-dlp aktualisiert", "yt-dlp mis à jour"),
        "setup.ytdlp_updated_toast" => (
            "yt-dlp updated successfully",
            "yt-dlp erfolgreich aktualisiert",
            "yt-dlp mis à jour avec succès",
        ),
        "setup.update_error" => ("Update error: {}", "Update-Fehler: {}", "Erreur de mise à jour : {}"),
        "setup.dl_deno" => ("Downloading Deno", "Lade Deno herunter", "Téléchargement de Deno"),
        "setup.dl_deno_status" => (
            "Downloading Deno ...",
            "Lade Deno herunter ...",
            "Téléchargement de Deno ...",
        ),
        "setup.deno_installed" => ("Deno installed", "Deno installiert", "Deno installé"),
        "setup.deno_installed_toast" => (
            "Deno (JS runtime) installed",
            "Deno (JS-Laufzeit) installiert",
            "Deno (moteur JS) installé",
        ),
        "setup.deno_error" => (
            "Deno installation error: {}",
            "Deno-Installationsfehler: {}",
            "Erreur d'installation de Deno : {}",
        ),
        "setup.progress_pct" => ("{} ... {}% ({} MiB)", "{} ... {}% ({} MiB)", "{} ... {} % ({} Mio)"),
        "setup.progress_plain" => ("{} ... {} MiB", "{} ... {} MiB", "{} ... {} Mio"),

        // ---------------- info tab ----------------
        "info.subtitle" => (
            "Version {}  ·  Rust / egui port",
            "Version {}  ·  Rust-/egui-Portierung",
            "Version {}  ·  portage Rust / egui",
        ),
        "info.f_download" => ("Download", "Download", "Téléchargement"),
        "info.f_download_d" => (
            "YouTube, TikTok, Instagram, X/Twitter and 1700+ more sites",
            "YouTube, TikTok, Instagram, X/Twitter und über 1700 weitere Seiten",
            "YouTube, TikTok, Instagram, X/Twitter et plus de 1700 autres sites",
        ),
        "info.f_antibot" => ("Anti-Bot", "Anti-Bot", "Anti-bot"),
        "info.f_antibot_d" => (
            "Optional --impersonate (curl_cffi) against 403/Cloudflare",
            "Optionales --impersonate (curl_cffi) gegen 403/Cloudflare",
            "Option --impersonate (curl_cffi) contre les erreurs 403/Cloudflare",
        ),
        "info.f_vegas" => ("Vegas Pro", "Vegas Pro", "Vegas Pro"),
        "info.f_vegas_d" => (
            "H.264/AAC preferred - directly compatible with Vegas Pro 23+",
            "H.264/AAC bevorzugt - direkt kompatibel mit Vegas Pro 23+",
            "H.264/AAC privilégiés - directement compatibles avec Vegas Pro 23+",
        ),
        "info.f_audio" => ("Audio", "Audio", "Audio"),
        "info.f_audio_d" => (
            "MP3 (CBR 320k), WAV/PCM or Opus",
            "MP3 (CBR 320k), WAV/PCM oder Opus",
            "MP3 (CBR 320k), WAV/PCM ou Opus",
        ),
        "info.f_quality" => ("Quality", "Qualität", "Qualité"),
        "info.f_quality_d" => (
            "4K, 1440p, 1080p, 720p, 480p - with AV1 preference",
            "4K, 1440p, 1080p, 720p, 480p - mit AV1-Präferenz",
            "4K, 1440p, 1080p, 720p, 480p - avec préférence AV1",
        ),
        "info.f_sponsorblock" => ("SponsorBlock", "SponsorBlock", "SponsorBlock"),
        "info.f_sponsorblock_d" => (
            "Automatically remove or mark sponsors",
            "Sponsoren automatisch entfernen oder markieren",
            "Supprimer ou marquer automatiquement les sponsors",
        ),
        "info.f_convert" => ("Convert", "Konvertieren", "Conversion"),
        "info.f_convert_d" => (
            "H.264, H.265, AV1 (SVT-AV1), ProRes 422, DNxHR, Vegas Sync Fix, MP3/WAV",
            "H.264, H.265, AV1 (SVT-AV1), ProRes 422, DNxHR, Vegas Sync Fix, MP3/WAV",
            "H.264, H.265, AV1 (SVT-AV1), ProRes 422, DNxHR, Vegas Sync Fix, MP3/WAV",
        ),
        "info.f_hardware" => ("Hardware", "Hardware", "Matériel"),
        "info.f_hardware_d" => (
            "NVIDIA NVENC, AMD AMF, Intel QSV, Auto-Detect",
            "NVIDIA NVENC, AMD AMF, Intel QSV, Auto-Erkennung",
            "NVIDIA NVENC, AMD AMF, Intel QSV, détection automatique",
        ),
        "info.f_hdr" => ("HDR/Color", "HDR/Farbe", "HDR/Couleur"),
        "info.f_hdr_d" => (
            "Color metadata is preserved during conversion",
            "Farb-Metadaten bleiben bei der Konvertierung erhalten",
            "Les métadonnées de couleur sont conservées lors de la conversion",
        ),
        "info.f_integrity" => ("Integrity", "Integrität", "Intégrité"),
        "info.f_integrity_d" => (
            "yt-dlp binaries verified against signed SHA2-256SUMS",
            "yt-dlp-Binaries werden gegen signierte SHA2-256SUMS geprüft",
            "Binaires yt-dlp vérifiés via les SHA2-256SUMS signés",
        ),

        // ---------------- conflict modal ----------------
        "conflict.title" => ("File already exists", "Datei existiert bereits", "Le fichier existe déjà"),
        "conflict.body" => (
            "\"{}\" is already in the target folder.",
            "„{}“ liegt bereits im Zielordner.",
            "« {} » se trouve déjà dans le dossier de destination.",
        ),
        "conflict.hint" => (
            "Save under a different name, overwrite, or skip.",
            "Unter anderem Namen speichern, überschreiben oder überspringen.",
            "Enregistrer sous un autre nom, écraser ou ignorer.",
        ),
        "conflict.save_as_label" => ("Save as:", "Speichern als:", "Enregistrer sous :"),
        "conflict.save_as" => ("Save as", "Speichern als", "Enregistrer sous"),
        "conflict.overwrite" => ("Overwrite", "Überschreiben", "Écraser"),
        "conflict.skip" => ("Skip", "Überspringen", "Ignorer"),
        "conflict.need_name" => (
            "Please enter a file name.",
            "Bitte einen Dateinamen eingeben.",
            "Veuillez saisir un nom de fichier.",
        ),
        "conflict.name_taken" => (
            "A file with this name already exists.",
            "Eine Datei mit diesem Namen existiert bereits.",
            "Un fichier portant ce nom existe déjà.",
        ),

        // ---------------- quality options ----------------
        "quality.best" => ("Best Quality (H.264)", "Beste Qualität (H.264)", "Meilleure qualité (H.264)"),
        "quality.best_av1" => (
            "Best Quality (AV1 preferred)",
            "Beste Qualität (AV1 bevorzugt)",
            "Meilleure qualité (AV1 préféré)",
        ),
        "quality.2160" => ("4K (2160p)", "4K (2160p)", "4K (2160p)"),
        "quality.1440" => ("1440p", "1440p", "1440p"),
        "quality.1080" => ("1080p", "1080p", "1080p"),
        "quality.720" => ("720p", "720p", "720p"),
        "quality.480" => ("480p", "480p", "480p"),
        "quality.audio_wav" => (
            "Audio Only (WAV - Vegas/NLE)",
            "Nur Audio (WAV - Vegas/NLE)",
            "Audio seul (WAV - Vegas/NLE)",
        ),
        "quality.audio" => ("Audio Only (MP3 320k)", "Nur Audio (MP3 320k)", "Audio seul (MP3 320k)"),
        "quality.audio_opus" => (
            "Audio Only (Opus, small)",
            "Nur Audio (Opus, klein)",
            "Audio seul (Opus, léger)",
        ),

        // ---------------- conflict options ----------------
        "conflictopt.ask" => (
            "Ask me (choose the name)",
            "Nachfragen (Namen wählen)",
            "Me demander (choisir le nom)",
        ),
        "conflictopt.rename" => (
            "Auto-rename - Title (1).mp4",
            "Automatisch umbenennen - Titel (1).mp4",
            "Renommer automatiquement - Titre (1).mp4",
        ),
        "conflictopt.overwrite" => (
            "Overwrite existing file",
            "Vorhandene Datei überschreiben",
            "Écraser le fichier existant",
        ),
        "conflictopt.skip" => (
            "Skip (keep existing file)",
            "Überspringen (vorhandene Datei behalten)",
            "Ignorer (conserver le fichier existant)",
        ),

        // ---------------- cookies options ----------------
        "cookies.none" => ("None (default)", "Keine (Standard)", "Aucun (par défaut)"),
        "cookies.firefox" => (
            "Firefox (recommended on Windows)",
            "Firefox (unter Windows empfohlen)",
            "Firefox (recommandé sous Windows)",
        ),
        "cookies.cookiefile" => (
            "Cookies File (cookies.txt)",
            "Cookies-Datei (cookies.txt)",
            "Fichier de cookies (cookies.txt)",
        ),
        "cookies.chrome" => (
            "Chrome (often blocked/Windows)",
            "Chrome (unter Windows oft blockiert)",
            "Chrome (souvent bloqué sous Windows)",
        ),
        "cookies.edge" => (
            "Edge (often blocked/Windows)",
            "Edge (unter Windows oft blockiert)",
            "Edge (souvent bloqué sous Windows)",
        ),
        "cookies.brave" => (
            "Brave (often blocked/Windows)",
            "Brave (unter Windows oft blockiert)",
            "Brave (souvent bloqué sous Windows)",
        ),
        "cookies.safari" => ("Safari (macOS)", "Safari (macOS)", "Safari (macOS)"),

        // ---------------- category options ----------------
        "cat.standard" => ("Standard", "Standard", "Standard"),
        "cat.editing" => ("Editing", "Bearbeitung", "Montage"),
        "cat.delivery" => ("Delivery", "Auslieferung", "Diffusion"),
        "cat.audio" => ("Audio (MP3 / WAV)", "Audio (MP3 / WAV)", "Audio (MP3 / WAV)"),

        // ---------------- hardware options ----------------
        "hw.auto" => ("Auto", "Automatisch", "Automatique"),
        "hw.nvidia" => ("NVIDIA NVENC", "NVIDIA NVENC", "NVIDIA NVENC"),
        "hw.amd" => ("AMD AMF", "AMD AMF", "AMD AMF"),
        "hw.intel" => ("Intel QSV", "Intel QSV", "Intel QSV"),
        "hw.cpu" => ("CPU", "CPU", "Processeur"),

        // ---------------- bitrate mode options ----------------
        "brmode.crf" => ("CRF / CQ", "CRF / CQ", "CRF / CQ"),
        "brmode.custom" => ("Custom Bitrate", "Eigene Bitrate", "Débit personnalisé"),

        // ---------------- channel options ----------------
        "channel.stable" => ("Stable (recommended)", "Stable (empfohlen)", "Stable (recommandé)"),
        "channel.nightly" => (
            "Nightly (latest fixes)",
            "Nightly (neueste Fixes)",
            "Nightly (derniers correctifs)",
        ),
        "channel.master" => ("Master (bleeding edge)", "Master (brandaktuell)", "Master (version de pointe)"),

        // ---------------- codec options ----------------
        "codec.h264" => ("H.264 (compatible)", "H.264 (kompatibel)", "H.264 (compatible)"),
        "codec.h265" => ("H.265 / HEVC", "H.265 / HEVC", "H.265 / HEVC"),
        "codec.av1" => ("AV1 (modern, small)", "AV1 (modern, klein)", "AV1 (moderne, léger)"),
        "codec.vp9" => ("VP9 (Web)", "VP9 (Web)", "VP9 (Web)"),
        "codec.copy" => ("Copy stream", "Stream kopieren", "Copier le flux"),
        "codec.h264_allintra" => ("H.264 All-Intra", "H.264 All-Intra", "H.264 All-Intra"),
        "codec.h264_handbrake" => ("H.264 Editing", "H.264 Bearbeitung", "H.264 montage"),
        "codec.vegas_fix" => ("Vegas Sync Fix", "Vegas Sync Fix", "Vegas Sync Fix"),
        "codec.prores422" => ("ProRes 422", "ProRes 422", "ProRes 422"),
        "codec.prores422hq" => ("ProRes 422 HQ", "ProRes 422 HQ", "ProRes 422 HQ"),
        "codec.dnxhr_hq" => ("DNxHR HQ", "DNxHR HQ", "DNxHR HQ"),
        "codec.youtube" => ("YouTube Export (H.264)", "YouTube-Export (H.264)", "Export YouTube (H.264)"),
        "codec.youtube_av1" => ("YouTube Export (AV1)", "YouTube-Export (AV1)", "Export YouTube (AV1)"),
        "codec.social" => ("Instagram / TikTok", "Instagram / TikTok", "Instagram / TikTok"),
        "codec.audio_mp3" => ("MP3 (320 kbps)", "MP3 (320 kbit/s)", "MP3 (320 kbit/s)"),
        "codec.audio_wav" => ("WAV (PCM 16-bit)", "WAV (PCM 16 Bit)", "WAV (PCM 16 bits)"),

        // ---------------- download worker ----------------
        "dlw.started" => (
            "=== Download started ===",
            "=== Download gestartet ===",
            "=== Téléchargement démarré ===",
        ),
        "dlw.warn_nojs" => (
            "Warning: no JS runtime (Deno) found - YouTube often only offers 360p without one. Setup tab -> 'Install Deno'.",
            "Warnung: Keine JS-Laufzeit (Deno) gefunden - ohne sie bietet YouTube oft nur 360p an. Reiter Einrichtung -> „Deno installieren“.",
            "Avertissement : aucun moteur JS (Deno) détecté - sans lui, YouTube ne propose souvent que du 360p. Onglet Installation -> « Installer Deno ».",
        ),
        "dlw.note_nocookies" => (
            "Note: without cookies or a PO token, YouTube may withhold some HD formats.",
            "Hinweis: Ohne Cookies oder PO Token hält YouTube manche HD-Formate zurück.",
            "Remarque : sans cookies ni jeton PO, YouTube peut retenir certains formats HD.",
        ),
        "dlw.warn_ig_old" => (
            "Warning: yt-dlp {} is too old for Instagram (empty media response bug, #17074). Setup tab -> 'Update yt-dlp'.",
            "Warnung: yt-dlp {} ist zu alt für Instagram (Bug „empty media response“, #17074). Reiter Einrichtung -> „yt-dlp aktualisieren“.",
            "Avertissement : yt-dlp {} est trop ancien pour Instagram (bogue « empty media response », #17074). Onglet Installation -> « Mettre à jour yt-dlp ».",
        ),
        "dlw.note_ig_cookies" => (
            "Note: Instagram often requires login cookies (cookies.txt is the most reliable).",
            "Hinweis: Instagram verlangt oft Login-Cookies (cookies.txt ist am zuverlässigsten).",
            "Remarque : Instagram exige souvent des cookies de connexion (cookies.txt est le plus fiable).",
        ),
        "dlw.warn_ig_impersonate" => (
            "Warning: browser impersonation is unavailable - Instagram usually blocks downloads without it.",
            "Warnung: Browser-Impersonation ist nicht verfügbar - Instagram blockiert Downloads ohne sie meist.",
            "Avertissement : l'impersonation du navigateur est indisponible - sans elle, Instagram bloque généralement les téléchargements.",
        ),
        "dlw.checking_target" => (
            "Checking target file ...",
            "Prüfe die Zieldatei ...",
            "Vérification du fichier cible ...",
        ),
        "dlw.no_target_name" => (
            "Note: could not determine the target file name in advance - if a file with the same name exists, yt-dlp will skip the download.",
            "Hinweis: Der Zieldateiname konnte nicht im Voraus ermittelt werden - existiert bereits eine Datei gleichen Namens, überspringt yt-dlp den Download.",
            "Remarque : impossible de déterminer le nom du fichier cible à l'avance - si un fichier du même nom existe, yt-dlp ignorera le téléchargement.",
        ),
        "dlw.target_file" => ("Target file: {}", "Zieldatei: {}", "Fichier cible : {}"),
        "dlw.exists_saving_as" => (
            "\"{}\" already exists - saving as \"{}\".",
            "„{}“ existiert bereits - speichere als „{}“.",
            "« {} » existe déjà - enregistrement sous « {} ».",
        ),
        "dlw.waiting_decision" => (
            "Waiting for your decision ...",
            "Warte auf deine Entscheidung ...",
            "En attente de votre décision ...",
        ),
        "dlw.overwriting" => (
            "Overwriting the existing \"{}\".",
            "Überschreibe die vorhandene Datei „{}“.",
            "Écrasement du fichier existant « {} ».",
        ),
        "dlw.saving_as" => ("Saving as \"{}\".", "Speichere als „{}“.", "Enregistrement sous « {} »."),
        "dlw.cancelled_log" => (
            "\n=== Download cancelled ===",
            "\n=== Download abgebrochen ===",
            "\n=== Téléchargement annulé ===",
        ),
        "dlw.skipped_status" => (
            "Skipped - file already exists",
            "Übersprungen - Datei existiert bereits",
            "Ignoré - le fichier existe déjà",
        ),
        "dlw.skipped_log" => (
            "\n=== Skipped - the existing file was kept ===",
            "\n=== Übersprungen - die vorhandene Datei wurde behalten ===",
            "\n=== Ignoré - le fichier existant a été conservé ===",
        ),
        "dlw.skipped_toast" => (
            "Skipped - the existing file was kept",
            "Übersprungen - die vorhandene Datei wurde behalten",
            "Ignoré - le fichier existant a été conservé",
        ),
        "dlw.nothing_status" => (
            "Nothing downloaded - file already exists",
            "Nichts heruntergeladen - Datei existiert bereits",
            "Rien téléchargé - le fichier existe déjà",
        ),
        "dlw.nothing_log" => (
            "\n=== Nothing downloaded - a file with this name already exists ===",
            "\n=== Nichts heruntergeladen - eine Datei mit diesem Namen existiert bereits ===",
            "\n=== Rien téléchargé - un fichier portant ce nom existe déjà ===",
        ),
        "dlw.nothing_tip" => (
            "Tip: set 'If file exists' to 'Ask me', 'Auto-rename' or 'Overwrite' to download it anyway.",
            "Tipp: Stelle „Wenn die Datei existiert“ auf „Nachfragen“, „Automatisch umbenennen“ oder „Überschreiben“, um trotzdem herunterzuladen.",
            "Astuce : réglez « Si le fichier existe » sur « Me demander », « Renommer automatiquement » ou « Écraser » pour le télécharger quand même.",
        ),
        "dlw.completed" => ("Download completed", "Download abgeschlossen", "Téléchargement terminé"),
        "dlw.success_log" => (
            "\n=== Download successful ===",
            "\n=== Download erfolgreich ===",
            "\n=== Téléchargement réussi ===",
        ),
        "dlw.success_toast" => ("Download successful", "Download erfolgreich", "Téléchargement réussi"),
        "dlw.failed" => ("Download failed", "Download fehlgeschlagen", "Échec du téléchargement"),
        "dlw.failed_log" => (
            "\n=== Download failed (code {}) ===",
            "\n=== Download fehlgeschlagen (Code {}) ===",
            "\n=== Échec du téléchargement (code {}) ===",
        ),
        "dlw.tip_instagram" => (
            "Tip: known Instagram \"empty media response\" bug (yt-dlp #17074), fixed in {}. Update yt-dlp in the Setup tab.",
            "Tipp: Bekannter Instagram-Bug „empty media response“ (yt-dlp #17074), behoben in {}. yt-dlp im Reiter Einrichtung aktualisieren.",
            "Astuce : bogue Instagram connu « empty media response » (yt-dlp #17074), corrigé dans {}. Mettez yt-dlp à jour dans l'onglet Installation.",
        ),
        "dlw.tip_formats" => (
            "Tip: the site withheld formats or wants a login. Set cookies (cookies.txt), install Deno, or update yt-dlp.",
            "Tipp: Die Seite hat Formate zurückgehalten oder verlangt einen Login. Cookies setzen (cookies.txt), Deno installieren oder yt-dlp aktualisieren.",
            "Astuce : le site a retenu des formats ou exige une connexion. Définissez des cookies (cookies.txt), installez Deno ou mettez yt-dlp à jour.",
        ),
        "dlw.tip_nojs" => (
            "Tip: no JS runtime detected. Setup tab -> 'Install Deno' (needed for the YouTube n-challenge).",
            "Tipp: Keine JS-Laufzeit erkannt. Reiter Einrichtung -> „Deno installieren“ (nötig für die YouTube-n-Challenge).",
            "Astuce : aucun moteur JS détecté. Onglet Installation -> « Installer Deno » (nécessaire pour le n-challenge YouTube).",
        ),
        "dlw.panic" => (
            "Error: internal download worker panic",
            "Fehler: Interner Absturz des Download-Workers",
            "Erreur : plantage interne du processus de téléchargement",
        ),
        "dlw.progress_status" => (
            "{}%  |  {}  |  ETA {}",
            "{}%  |  {}  |  Restzeit {}",
            "{} %  |  {}  |  Temps restant {}",
        ),
        "dlw.downloading_pct" => (
            "Downloading ... {}%",
            "Lade herunter ... {}%",
            "Téléchargement ... {} %",
        ),

        // ---------------- convert worker ----------------
        "cvw.started" => (
            "=== Conversion started ===",
            "=== Konvertierung gestartet ===",
            "=== Conversion démarrée ===",
        ),
        "cvw.codec_audio" => (
            "Codec:  {} -> {} (audio only)",
            "Codec:  {} -> {} (nur Audio)",
            "Codec :  {} -> {} (audio seul)",
        ),
        "cvw.codec" => ("Codec:  {} -> {}", "Codec:  {} -> {}", "Codec :  {} -> {}"),
        "cvw.hw_auto" => ("HW:     auto -> {}", "HW:     auto -> {}", "Matériel : auto -> {}"),
        "cvw.mode" => ("Mode:   {}", "Modus:  {}", "Mode :  {}"),
        "cvw.mode_custom" => ("Custom {}M", "Eigene {}M", "Personnalisé {}M"),
        "cvw.mode_crf" => ("CRF {}", "CRF {}", "CRF {}"),
        "cvw.source" => ("Source: pix_fmt={}", "Quelle: pix_fmt={}", "Source : pix_fmt={}"),
        "cvw.color" => ("Color:  {}", "Farbe:  {}", "Couleur : {}"),
        "cvw.duration" => ("Duration: {}s", "Dauer: {}s", "Durée : {} s"),
        "cvw.converting_pct" => (
            "Converting ... {}%  |  {}  |  {}  |  {} fps  |  ETA {}",
            "Konvertiere ... {}%  |  {}  |  {}  |  {} fps  |  Restzeit {}",
            "Conversion ... {} %  |  {}  |  {}  |  {} fps  |  Temps restant {}",
        ),
        "cvw.converting_plain" => (
            "Converting ...  |  {}  |  {}  |  {} fps",
            "Konvertiere ...  |  {}  |  {}  |  {} fps",
            "Conversion ...  |  {}  |  {}  |  {} fps",
        ),
        "cvw.cancelled_log" => (
            "\n=== Conversion cancelled ===",
            "\n=== Konvertierung abgebrochen ===",
            "\n=== Conversion annulée ===",
        ),
        "cvw.completed" => ("Conversion completed", "Konvertierung abgeschlossen", "Conversion terminée"),
        "cvw.success_log" => (
            "\n=== Conversion successful ===",
            "\n=== Konvertierung erfolgreich ===",
            "\n=== Conversion réussie ===",
        ),
        "cvw.success_toast" => ("Conversion successful", "Konvertierung erfolgreich", "Conversion réussie"),
        "cvw.failed" => ("Conversion failed", "Konvertierung fehlgeschlagen", "Échec de la conversion"),
        "cvw.failed_log" => (
            "\n=== Conversion failed (code {}) ===",
            "\n=== Konvertierung fehlgeschlagen (Code {}) ===",
            "\n=== Échec de la conversion (code {}) ===",
        ),
        "cvw.panic" => (
            "Error: internal conversion worker panic",
            "Fehler: Interner Absturz des Konvertierungs-Workers",
            "Erreur : plantage interne du processus de conversion",
        ),

        // Unknown key: surface it rather than rendering nothing.
        _ => (key, key, key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts;

    #[test]
    fn unknown_key_falls_back_to_itself() {
        assert_eq!(t(Lang::De, "no.such.key"), "no.such.key");
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(
            tf(Lang::En, "dlw.exists_saving_as", &["a.mp4", "a (1).mp4"]),
            "\"a.mp4\" already exists - saving as \"a (1).mp4\"."
        );
    }

    #[test]
    fn argument_containing_a_placeholder_is_not_rescanned() {
        // A file literally named "{}" must not swallow the second slot.
        assert_eq!(
            tf(Lang::En, "dlw.exists_saving_as", &["{}", "b.mp4"]),
            "\"{}\" already exists - saving as \"b.mp4\"."
        );
    }

    #[test]
    fn missing_arguments_leave_the_placeholder_visible() {
        assert_eq!(tf(Lang::En, "cvw.mode_crf", &[]), "CRF {}");
    }

    /// Every dropdown label is a key; a typo would otherwise silently render
    /// as the raw key in the UI.
    #[test]
    fn every_option_table_key_is_translated() {
        let mut tables: Vec<&[(&'static str, &'static str)]> = vec![
            consts::QUALITY_OPTIONS,
            consts::CONFLICT_OPTIONS,
            consts::COOKIES_OPTIONS,
            consts::CATEGORY_OPTIONS,
            consts::HW_OPTIONS,
            consts::BITRATE_MODE_OPTIONS,
            consts::CHANNEL_OPTIONS,
        ];
        for (category, _) in consts::CATEGORY_OPTIONS {
            tables.push(consts::codec_options(category));
        }

        for table in tables {
            for (value, key) in table {
                for lang in LANGS {
                    assert_ne!(
                        t(lang, key),
                        *key,
                        "missing {:?} translation for option '{value}' (key '{key}')",
                        lang
                    );
                }
            }
        }
    }
}

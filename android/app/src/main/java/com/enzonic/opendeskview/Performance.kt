package com.enzonic.opendeskview

/**
 * Resource-usage presets sent to the server in the identification message. The server treats its
 * own settings as a ceiling, so these can lower its frame rate and quality but never raise them.
 */
enum class Performance(val label: String, val fps: Int, val quality: Int) {
    BATTERY("Battery saver — 24 fps, lower quality", 24, 50),
    BALANCED("Balanced — 30 fps", 30, 80),
    SMOOTHEST("Smoothest — 60 fps, best quality", 60, 100),
}

// Generated from design/tokens/tokens.json by packages/tokens/scripts/generate.mjs. Do not edit.
package sh.hoi.core.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/** hoi colour tokens. Text on pastels, sun and coral is always [HoiColors.tagInk], in both themes. */
data class HoiColors(
    val bg: Color,
    val card: Color,
    val ink: Color,
    val mute: Color,
    val line: Color,
    val soft: Color,
    val onink: Color,
    val coral: Color,
    val cobalt: Color,
    val sun: Color,
    val mint: Color,
    val sky: Color,
    val lilac: Color,
    val peach: Color,
    val tagInk: Color = Color(0xFF1A1A1A),
)

val HoiLight = HoiColors(
    bg = Color(0xFFFBFAF5),
    card = Color(0xFFFFFFFF),
    ink = Color(0xFF1A1A1A),
    mute = Color(0xFF6A6A63),
    line = Color(0xFFD9D7CC),
    soft = Color(0xFFF0EEE4),
    onink = Color(0xFFFFFFFF),
    coral = Color(0xFFFF6B4A),
    cobalt = Color(0xFF2F55E6),
    sun = Color(0xFFFFE16A),
    mint = Color(0xFFBFF0D0),
    sky = Color(0xFFCFE6FF),
    lilac = Color(0xFFDCD2FF),
    peach = Color(0xFFFFD2C2),
)

val HoiDark = HoiColors(
    bg = Color(0xFF15151A),
    card = Color(0xFF1E1E24),
    ink = Color(0xFFF4F2EA),
    mute = Color(0xFFA3A39A),
    line = Color(0xFF34343B),
    soft = Color(0xFF26262D),
    onink = Color(0xFF15151A),
    coral = Color(0xFFFF6B4A),
    cobalt = Color(0xFF8AA0FF),
    sun = Color(0xFFFFE16A),
    mint = Color(0xFFBFF0D0),
    sky = Color(0xFFCFE6FF),
    lilac = Color(0xFFDCD2FF),
    peach = Color(0xFFFFD2C2),
)

/** Community palette, in the order communities pick from. */
val CommunityPalette = listOf("mint", "sky", "lilac", "peach", "sun", "coral")

object HoiSpace {
    val s1 = 4.dp
    val s2 = 8.dp
    val s3 = 12.dp
    val s4 = 16.dp
    val s5 = 20.dp
    val s6 = 24.dp
    val s8 = 32.dp
    val s10 = 40.dp
    val s14 = 56.dp
    val s18 = 72.dp
}

object HoiRadius {
    val tag = 999.dp
    val pill = 999.dp
    val card = 20.dp
    val frame = 24.dp
    val input = 14.dp
    val poof = 14.dp
}

object HoiMotion {
    const val microMs = 300
    const val uiMs = 420
    const val enterMs = 520
    const val heroMs = 900
    const val staggerRowsMs = 45
    const val staggerWordsMs = 55
    const val staggerNodesMs = 40
}

const val HoiDisplayFont = "Gabarito"
const val HoiMonoFont = "Geist Mono"

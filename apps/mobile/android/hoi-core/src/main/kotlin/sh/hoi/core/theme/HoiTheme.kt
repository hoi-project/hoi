package sh.hoi.core.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

/**
 * The two typefaces (design/tokens/tokens.json). Bundle Gabarito and Geist Mono (both OFL) in your app's
 * `res/font/` and pass them in, e.g. `HoiFonts(display = FontFamily(Font(R.font.gabarito)))`.
 * Never use downloadable fonts: that asks a Google service for them on every phone.
 */
@Immutable
data class HoiFonts(
    val display: FontFamily = FontFamily.SansSerif,
    val mono: FontFamily = FontFamily.Monospace,
)

private val LocalHoiColors = staticCompositionLocalOf { HoiLight }
private val LocalHoiFonts = staticCompositionLocalOf { HoiFonts() }

/**
 * hoi's look: the warm off-white ground, ink outlines and pastel fills from design/tokens,
 * light and dark (a full dark set, not an inversion). Material components pick up the closest
 * roles; hoi's own components read [HoiTheme.colors] directly.
 */
@Composable
fun HoiTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    fonts: HoiFonts = HoiFonts(),
    content: @Composable () -> Unit,
) {
    val c = if (darkTheme) HoiDark else HoiLight
    val scheme = if (darkTheme) {
        darkColorScheme(
            primary = c.ink, onPrimary = c.onink,
            secondary = c.coral, onSecondary = c.tagInk,
            tertiary = c.cobalt, onTertiary = c.bg,
            background = c.bg, onBackground = c.ink,
            surface = c.bg, onSurface = c.ink,
            surfaceVariant = c.soft, onSurfaceVariant = c.mute,
            outline = c.ink, outlineVariant = c.line,
            error = c.coral, onError = c.tagInk,
        )
    } else {
        lightColorScheme(
            primary = c.ink, onPrimary = c.onink,
            secondary = c.coral, onSecondary = c.tagInk,
            tertiary = c.cobalt, onTertiary = c.bg,
            background = c.bg, onBackground = c.ink,
            surface = c.bg, onSurface = c.ink,
            surfaceVariant = c.soft, onSurfaceVariant = c.mute,
            outline = c.ink, outlineVariant = c.line,
            error = c.coral, onError = c.tagInk,
        )
    }
    val d = fonts.display
    // design/tokens type scale at phone sizes (the web clamps to these minimums).
    val typography = Typography(
        displayLarge = TextStyle(fontFamily = d, fontWeight = FontWeight.Black, fontSize = 44.sp, lineHeight = 0.93.em, letterSpacing = (-0.045).em),
        headlineLarge = TextStyle(fontFamily = d, fontWeight = FontWeight.Black, fontSize = 34.sp, lineHeight = 1.02.em, letterSpacing = (-0.035).em),
        headlineMedium = TextStyle(fontFamily = d, fontWeight = FontWeight.Black, fontSize = 30.sp, lineHeight = 1.0.em, letterSpacing = (-0.035).em),
        titleLarge = TextStyle(fontFamily = d, fontWeight = FontWeight.ExtraBold, fontSize = 19.sp, lineHeight = 1.15.em, letterSpacing = (-0.02).em),
        bodyLarge = TextStyle(fontFamily = d, fontWeight = FontWeight.Normal, fontSize = 17.sp, lineHeight = 1.5.em),
        bodyMedium = TextStyle(fontFamily = d, fontWeight = FontWeight.Normal, fontSize = 16.sp, lineHeight = 1.45.em),
        labelLarge = TextStyle(fontFamily = d, fontWeight = FontWeight.Bold, fontSize = 15.sp),
        labelMedium = TextStyle(fontFamily = d, fontWeight = FontWeight.Medium, fontSize = 14.sp),
        labelSmall = TextStyle(fontFamily = d, fontWeight = FontWeight.Bold, fontSize = 13.sp),
    )
    CompositionLocalProvider(LocalHoiColors provides c, LocalHoiFonts provides fonts) {
        MaterialTheme(colorScheme = scheme, typography = typography, content = content)
    }
}

/** hoi's own tokens inside [HoiTheme]: `HoiTheme.colors.coral`, `HoiTheme.fonts.mono`. */
object HoiTheme {
    val colors: HoiColors
        @Composable @ReadOnlyComposable
        get() = LocalHoiColors.current

    val fonts: HoiFonts
        @Composable @ReadOnlyComposable
        get() = LocalHoiFonts.current

    /** Geist Mono, 12 sp, for fingerprints, record ids and hashes only. Use with `colors.mute`. */
    val mono: TextStyle
        @Composable @ReadOnlyComposable
        get() = TextStyle(fontFamily = LocalHoiFonts.current.mono, fontSize = 12.sp)

    /**
     * A community's colour by name ("mint", "sky", "lilac", "peach", "sun", "coral"). Text on it
     * is always [HoiColors.tagInk], in both themes.
     */
    @Composable
    @ReadOnlyComposable
    fun community(name: String): Color = with(LocalHoiColors.current) {
        when (name) {
            "mint" -> mint
            "sky" -> sky
            "lilac" -> lilac
            "peach" -> peach
            "sun" -> sun
            "coral" -> coral
            else -> soft
        }
    }
}

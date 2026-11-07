// Generated from design/tokens/tokens.json by packages/tokens/scripts/generate.mjs. Do not edit.
import SwiftUI

/// hoi colour tokens. Each colour resolves for light and dark appearance like the web (`prefers-color-scheme`).
/// Text on pastels, `sun` and `coral` is always `HoiColor.tagInk`, in both themes.
public enum HoiColor {
    public static let bg = Color(light: 0xFBFAF5, dark: 0x15151A)
    public static let card = Color(light: 0xFFFFFF, dark: 0x1E1E24)
    public static let ink = Color(light: 0x1A1A1A, dark: 0xF4F2EA)
    public static let mute = Color(light: 0x6A6A63, dark: 0xA3A39A)
    public static let line = Color(light: 0xD9D7CC, dark: 0x34343B)
    public static let soft = Color(light: 0xF0EEE4, dark: 0x26262D)
    public static let onink = Color(light: 0xFFFFFF, dark: 0x15151A)
    public static let coral = Color(light: 0xFF6B4A, dark: 0xFF6B4A)
    public static let cobalt = Color(light: 0x2F55E6, dark: 0x8AA0FF)
    public static let sun = Color(light: 0xFFE16A, dark: 0xFFE16A)
    public static let mint = Color(light: 0xBFF0D0, dark: 0xBFF0D0)
    public static let sky = Color(light: 0xCFE6FF, dark: 0xCFE6FF)
    public static let lilac = Color(light: 0xDCD2FF, dark: 0xDCD2FF)
    public static let peach = Color(light: 0xFFD2C2, dark: 0xFFD2C2)
    public static let tagInk = Color(hex: 0x1A1A1A)
    /// Community palette, in the order communities pick from.
    public static let communityPalette: [String: Color] = ["mint": mint, "sky": sky, "lilac": lilac, "peach": peach, "sun": sun, "coral": coral]
}

public enum HoiSpace {
    public static let s1: CGFloat = 4
    public static let s2: CGFloat = 8
    public static let s3: CGFloat = 12
    public static let s4: CGFloat = 16
    public static let s5: CGFloat = 20
    public static let s6: CGFloat = 24
    public static let s8: CGFloat = 32
    public static let s10: CGFloat = 40
    public static let s14: CGFloat = 56
    public static let s18: CGFloat = 72
}

public enum HoiRadius {
    public static let tag: CGFloat = 999
    public static let pill: CGFloat = 999
    public static let card: CGFloat = 20
    public static let frame: CGFloat = 24
    public static let input: CGFloat = 14
    public static let poof: CGFloat = 14
}

public enum HoiFont {
    public static let display = "Gabarito"
    public static let mono = "Geist Mono"
}

public enum HoiMotion {
    public static let micro: Double = 0.3
    public static let ui: Double = 0.42
    public static let enter: Double = 0.52
    public static let hero: Double = 0.9
    public static let staggerRows: Double = 0.045
    public static let staggerWords: Double = 0.055
    public static let staggerNodes: Double = 0.04
}

extension Color {
    init(hex: UInt32) {
        self.init(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255, blue: Double(hex & 0xFF) / 255, opacity: 1)
    }
    init(light: UInt32, dark: UInt32) {
        #if canImport(UIKit)
        self.init(UIColor { $0.userInterfaceStyle == .dark ? UIColor(Color(hex: dark)) : UIColor(Color(hex: light)) })
        #else
        self.init(hex: light)
        #endif
    }
}

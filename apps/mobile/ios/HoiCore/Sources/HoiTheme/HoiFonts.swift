import CoreText
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Fonts (design/tokens/tokens.json): Gabarito for everything readable, Geist Mono only for key fingerprints,
// record ids and hashes. Both are SIL Open Font License; ship them inside the app, never load
// them from the network.
//
// 1. Add the TTF files to the app target: Gabarito (variable, 400–900) and Geist Mono
//    (400, 500), both on Google Fonts; Geist Mono also at github.com/vercel/geist-font. The web
//    packages in node_modules ship woff2, which iOS can't register.
// 2. Either list them in Info.plist under `UIAppFonts`, or call `HoiFonts.register()` once at
//    launch to register every .ttf/.otf in the main bundle.
// 3. Use `Font.hoiDisplay(...)` / `Font.hoiMono(...)`. Until the fonts are there they fall back
//    to the system font, so nothing breaks while you set things up.

public enum HoiFonts {
    /// Registers every .ttf and .otf in `bundle` for this process. Safe to call more than once.
    public static func register(bundle: Bundle = .main) {
        let urls = ["ttf", "otf"].flatMap { bundle.urls(forResourcesWithExtension: $0, subdirectory: nil) ?? [] }
        for url in urls {
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }

    /// The registered face to ask for: `Font.custom` wants a face name ("Gabarito-Regular"),
    /// not the family; `.weight(_:)` then picks the weight.
    static var display: String { face(HoiFont.display) }
    static var mono: String { face(HoiFont.mono) }

    private static func face(_ family: String) -> String {
        #if canImport(UIKit)
        return UIFont.fontNames(forFamilyName: family).first ?? family
        #else
        return family
        #endif
    }
}

extension Font {
    /// Gabarito at `size`, scaling with Dynamic Type relative to `style`. Headlines use `.black`
    /// (900) with tight tracking: `.tracking(-0.035 * size)` (see `HoiTracking`).
    public static func hoiDisplay(size: CGFloat, weight: Font.Weight = .regular, relativeTo style: Font.TextStyle = .body) -> Font {
        .custom(HoiFonts.display, size: size, relativeTo: style).weight(weight)
    }

    /// Geist Mono for fingerprints, ids and hashes: 12 pt, muted, nothing else.
    public static func hoiMono(size: CGFloat = 12, weight: Font.Weight = .regular) -> Font {
        .custom(HoiFonts.mono, size: size, relativeTo: .caption).weight(weight)
    }
}

/// Letter spacing from the type scale, as fractions of the font size.
public enum HoiTracking {
    /// The hero: -0.045em.
    public static let hero: CGFloat = -0.045
    /// h1 / h2 headlines: -0.035em.
    public static let headline: CGFloat = -0.035
    /// Post titles: -0.02em.
    public static let title: CGFloat = -0.02
}

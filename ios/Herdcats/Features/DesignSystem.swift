import Observation
import SwiftUI
import UIKit

// MARK: - Design System

/// Central design system tokens, themes, typography, shapes, and view modifiers for HerdrCat.
/// Conforms to the visual specification described in `design.md`.
enum DesignSystem {

    // MARK: - Corner Radii

    /// DESIGN.md §5 uses three UI radii: 8pt controls, 16pt cards, and pills.
    /// The named steps below map onto those so call sites keep their intent.
    enum CornerRadius {
        /// 0pt — edge-to-edge raw terminal output
        static let none: CGFloat = 0
        /// 7pt — app-icon logo thumbnails, echoing the iOS icon mask
        static let xs: CGFloat = 7
        /// 8pt — compact drawer buttons, follow-up chips
        static let sm: CGFloat = 8
        /// 8pt — standard text inputs, worktree rows
        static let md: CGFloat = 8
        /// 16pt — error banners, code blocks, pane switcher cards
        static let lg: CGFloat = 16
        /// 16pt — primary workspace cards and agent triage cards
        static let xl: CGFloat = 16
        /// 16pt — recent connection cards, connection setup sheet
        static let xxl: CGFloat = 16
        /// 8pt — primary and secondary buttons
        static let button: CGFloat = 8
        /// 9999pt — status pills, agent badges, circular action buttons
        static let full: CGFloat = 9999
    }

    // MARK: - Spacing Scale

    enum Spacing {
        /// 4pt — micro gap between icon and text inside a badge
        static let xs: CGFloat = 4
        /// 8pt — standard horizontal spacing in pills, tight vertical stacks
        static let sm: CGFloat = 8
        /// 12pt — chip padding, form field vertical insets, error banner margins
        static let md: CGFloat = 12
        /// 16pt — standard screen margin, card container padding, default spacing
        static let base: CGFloat = 16
        /// 20pt — vertical spacing between sections
        static let lg: CGFloat = 20
        /// 24pt — major group separations
        static let xl: CGFloat = 24
        /// 32pt — screen top/bottom gutters
        static let xxl: CGFloat = 32
    }
}

// MARK: - Theme

/// Raw "Moonlit" ramp from `DESIGN.md` §2. Views use the semantic `Theme`
/// tokens below, never these directly, so night and day come from one place.
enum Palette {
    static let night950: UInt32 = 0x1B282E
    static let night900: UInt32 = 0x26343D
    static let night800: UInt32 = 0x325156
    static let teal600: UInt32 = 0x4C777D
    static let teal400: UInt32 = 0x63988E
    static let mint200: UInt32 = 0xABE0B6
    static let paper50: UInt32 = 0xF8F8F8
    static let lacquer900: UInt32 = 0x4B221C
}

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}

// MARK: - Theme preferences

enum AppPalette: String, CaseIterable, Identifiable {
    case moonlit, latte, frappe, macchiato, mocha, solarizedLight, solarizedDark

    var id: String { rawValue }
    var title: String {
        switch self {
        case .moonlit: "Moonlit"
        case .latte: "Catppuccin Latte"
        case .frappe: "Catppuccin Frappé"
        case .macchiato: "Catppuccin Macchiato"
        case .mocha: "Catppuccin Mocha"
        case .solarizedLight: "Solarized Light"
        case .solarizedDark: "Solarized Dark"
        }
    }

    static let lightChoices: [Self] = [.moonlit, .latte, .solarizedLight]
    static let darkChoices: [Self] = [.moonlit, .frappe, .macchiato, .mocha, .solarizedDark]
    static let lightStorageKey = "lightThemePalette"
    static let darkStorageKey = "darkThemePalette"

    static func restored(_ raw: String?, dark: Bool) -> Self {
        let choices = dark ? darkChoices : lightChoices
        guard let raw, let palette = Self(rawValue: raw), choices.contains(palette) else {
            return .moonlit
        }
        return palette
    }

    /// Catppuccin palette 1.8.0, https://github.com/catppuccin/palette (MIT).
    fileprivate var colors: [String: UInt32] {
        switch self {
        case .moonlit, .solarizedLight, .solarizedDark: [:]
        case .latte:
            [
                "base": 0xEFF1F5,
                "mantle": 0xE6E9EF,
                "crust": 0xDCE0E8,
                "surface0": 0xCCD0DA,
                "surface1": 0xBCC0CC,
                "text": 0x4C4F69,
                "subtext1": 0x5C5F77,
                "mauve": 0x8839EF,
                "red": 0xD20F39,
                "green": 0x40A02B,
                "yellow": 0xDF8E1D,
                "teal": 0x179299,
                "surface2": 0xACB0BE,
                "subtext0": 0x6C6F85,
                "blue": 0x1E66F5,
                "pink": 0xEA76CB
            ]
        case .frappe:
            [
                "base": 0x303446,
                "mantle": 0x292C3C,
                "crust": 0x232634,
                "surface0": 0x414559,
                "surface1": 0x51576D,
                "text": 0xC6D0F5,
                "subtext1": 0xB5BFE2,
                "mauve": 0xCA9EE6,
                "red": 0xE78284,
                "green": 0xA6D189,
                "yellow": 0xE5C890,
                "teal": 0x81C8BE,
                "surface2": 0x626880,
                "subtext0": 0xA5ADCE,
                "blue": 0x8CAAEE,
                "pink": 0xF4B8E4
            ]
        case .macchiato:
            [
                "base": 0x24273A,
                "mantle": 0x1E2030,
                "crust": 0x181926,
                "surface0": 0x363A4F,
                "surface1": 0x494D64,
                "text": 0xCAD3F5,
                "subtext1": 0xB8C0E0,
                "mauve": 0xC6A0F6,
                "red": 0xED8796,
                "green": 0xA6DA95,
                "yellow": 0xEED49F,
                "teal": 0x8BD5CA,
                "surface2": 0x5B6078,
                "subtext0": 0xA5ADCB,
                "blue": 0x8AADF4,
                "pink": 0xF5BDE6
            ]
        case .mocha:
            [
                "base": 0x1E1E2E,
                "mantle": 0x181825,
                "crust": 0x11111B,
                "surface0": 0x313244,
                "surface1": 0x45475A,
                "text": 0xCDD6F4,
                "subtext1": 0xBAC2DE,
                "mauve": 0xCBA6F7,
                "red": 0xF38BA8,
                "green": 0xA6E3A1,
                "yellow": 0xF9E2AF,
                "teal": 0x94E2D5,
                "surface2": 0x585B70,
                "subtext0": 0xA6ADC8,
                "blue": 0x89B4FA,
                "pink": 0xF5C2E7
            ]
        }
    }
}

/// Immutable terminal rendering input and cache key. UI and terminal colors
/// share the selected palette, while explicit RGB/extended ANSI remain intact.
struct TerminalPalette: Equatable {
    let colors: [UInt32]
    let foreground: UInt32
    let background: UInt32

    init(palette: AppPalette, dark: Bool) {
        foreground = Self.hex(Theme.paletteColor("text", palette: palette, dark: dark,
                                                fallback: dark ? Palette.paper50 : Palette.night950))
        background = Self.hex(Theme.paletteColor("background", palette: palette, dark: dark,
                                                fallback: dark ? Palette.night950 : 0xDCEBE2))
        switch palette {
        case .moonlit:
            if dark {
                colors = [0x1B282E, 0xF29A86, 0xABE0B6, 0xE0B866, 0x8BB9D9, 0xC6A0C9, 0x8CC2B6, 0xD1DDD7,
                          0x63988E, 0xFFC0AD, 0xC9EED0, 0xF0D39A, 0xB5D5EB, 0xDFC3E1, 0xB5DED2, 0xF8F8F8]
            } else {
                colors = [0x1B282E, 0xA8432F, 0x2F7148, 0x865C0E, 0x326688, 0x795080, 0x325156, 0xB4C9BE,
                          0x4C777D, 0xB84D37, 0x387E52, 0x946715, 0x3C7296, 0x885B90, 0x3B6970, 0xF8FBF9]
            }
        case .solarizedLight, .solarizedDark:
            // Solarized's canonical ANSI order intentionally places orange and
            // violet in the bright red/magenta slots, and base tones elsewhere.
            colors = [0x073642, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5,
                      0x002B36, 0xCB4B16, 0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3]
        default:
            // Catppuccin's official Kitty ANSI mapping, using palette 1.8.0.
            let c = palette.colors
            let black = palette == .latte ? c["subtext1"]! : c["surface1"]!
            let brightBlack = palette == .latte ? c["subtext0"]! : c["surface2"]!
            let white = palette == .latte ? c["surface2"]! : c["subtext1"]!
            let brightWhite = palette == .latte ? c["surface1"]! : c["subtext0"]!
            let accents = [c["red"]!, c["green"]!, c["yellow"]!, c["blue"]!, c["pink"]!, c["teal"]!]
            colors = [black] + accents + [white, brightBlack] + accents + [brightWhite]
        }
    }

    private static func hex(_ color: UIColor) -> UInt32 {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        color.getRed(&r, green: &g, blue: &b, alpha: &a)
        return UInt32((r * 255).rounded()) << 16 | UInt32((g * 255).rounded()) << 8 | UInt32((b * 255).rounded())
    }
}

/// Shared observable palette choices. Reading Theme tokens registers a SwiftUI
/// dependency, so a selection redraws existing screens without recreating them.
@Observable
final class AppThemePreferences {
    static let shared = AppThemePreferences()
    private let defaults: UserDefaults
    private(set) var light: AppPalette
    private(set) var dark: AppPalette

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        light = AppPalette.restored(defaults.string(forKey: AppPalette.lightStorageKey), dark: false)
        dark = AppPalette.restored(defaults.string(forKey: AppPalette.darkStorageKey), dark: true)
    }

    func select(_ palette: AppPalette, dark isDark: Bool) {
        guard (isDark ? AppPalette.darkChoices : AppPalette.lightChoices).contains(palette) else { return }
        if isDark {
            dark = palette
            defaults.set(palette.rawValue, forKey: AppPalette.darkStorageKey)
        } else {
            light = palette
            defaults.set(palette.rawValue, forKey: AppPalette.lightStorageKey)
        }
    }
}

enum Theme {
    /// Stable Color values preserve equality and avoid creating UIKit providers
    /// for every view update. The cache has no mutable shared state.
    private static let colors: [String: Color] = {
        let tokens: [(String, UInt32, UInt32, CGFloat, CGFloat)] = [
            ("accent", Palette.mint200, Palette.night800, 1, 1),
            ("accentFill", Palette.mint200, Palette.night800, 1, 1),
            ("sky", Palette.teal600, 0xD3EBDD, 1, 1),
            ("background", Palette.night950, 0xDCEBE2, 1, 1),
            ("field", Palette.night950, 0xEEF6F1, 1, 1),
            ("card", 0x31464E, 0xF8FBF9, 1, 1),
            ("border", Palette.night800, 0xC5DCCF, 1, 1),
            ("separator", 0x28343A, 0xCBDAD2, 1, 1),
            ("raised", 0x446670, 0xC5DCCF, 1, 1),
            ("text", Palette.paper50, Palette.night950, 1, 1),
            ("muted", Palette.paper50, Palette.night950, 0.72, 0.72),
            ("warm", Palette.lacquer900, 0x8A3324, 1, 1),
            ("idle", 0x8CC2B6, Palette.teal600, 1, 1),
            ("done", Palette.mint200, 0x2F7148, 1, 1),
            ("working", 0xE0B866, 0x865C0E, 1, 1),
            ("blocked", 0xF29A86, 0xA8432F, 1, 1),
            ("addition", Palette.mint200, Palette.mint200, 0.22, 0.6),
            ("deletion", 0xF29A86, 0xA8432F, 0.24, 0.18),
            ("onPrimary", Palette.night950, Palette.paper50, 1, 1)
        ]
        var result: [String: Color] = [:]
        for light in AppPalette.lightChoices {
            for dark in AppPalette.darkChoices {
                for (token, night, day, nightAlpha, dayAlpha) in tokens {
                    let darkColor = paletteColor(token, palette: dark, dark: true, fallback: night, alpha: nightAlpha)
                    let lightColor = paletteColor(token, palette: light, dark: false, fallback: day, alpha: dayAlpha)
                    result["\(light.rawValue)/\(dark.rawValue)/\(token)"] = Color(uiColor: UIColor {
                        $0.userInterfaceStyle == .dark ? darkColor : lightColor
                    })
                }
            }
        }
        return result
    }()

    private static func color(_ token: String) -> Color {
        let preferences = AppThemePreferences.shared
        return colors["\(preferences.light.rawValue)/\(preferences.dark.rawValue)/\(token)"]!
    }

    static func paletteColor(_ token: String, palette: AppPalette, dark: Bool,
                             fallback: UInt32, alpha: CGFloat = 1) -> UIColor {
        guard palette != .moonlit else { return UIColor(hex: fallback, alpha: alpha) }
        if palette == .solarizedLight || palette == .solarizedDark {
            return solarizedColor(token, dark: palette == .solarizedDark, alpha: alpha)
        }
        // List separators keep a consistent, gentle contrast across palettes.
        if token == "separator" {
            let separators: [AppPalette: UInt32] = [
                .latte: 0xD4D7DF, .frappe: 0x35384A,
                .macchiato: 0x2B2D3F, .mocha: 0x262736
            ]
            return UIColor(hex: separators[palette]!)
        }
        let colors = palette.colors
        let key: String
        switch token {
        case "accent", "accentFill": key = "mauve"
        case "background", "field": key = "mantle"
        case "card", "sky": key = "base"
        case "border", "raised": key = "surface0"
        case "text": key = "text"
        case "muted": key = "subtext1"
        case "idle": key = "teal"
        case "done", "addition": key = "green"
        case "working": key = "yellow"
        case "blocked", "deletion", "warm": key = "red"
        case "onPrimary": key = dark ? "crust" : "base"
        default: key = "text"
        }
        // Accent text on a light card needs deeper status colors than Latte's
        // bright green/yellow/teal. Keep the hue, lowering luminance for AA.
        let lightStatus: [String: UInt32] = ["idle": 0x13777D, "done": 0x2C7A1F, "working": 0x8A5700]
        let hex = palette == .latte ? (lightStatus[token] ?? colors[key]!) : colors[key]!
        let effectiveAlpha: CGFloat
        if token == "muted" {
            effectiveAlpha = 1
        } else if palette == .latte && (token == "addition" || token == "deletion") {
            effectiveAlpha = 0.12
        } else {
            effectiveAlpha = alpha
        }
        return UIColor(hex: hex, alpha: effectiveAlpha)
    }

    /// Official Solarized bases; accents are tuned for AA on the app's cards.
    /// https://ethanschoonover.com/solarized/ (MIT, Ethan Schoonover).
    private static func solarizedColor(_ token: String, dark: Bool, alpha: CGFloat) -> UIColor {
        let hex: UInt32
        switch token {
        case "background", "field": hex = dark ? 0x002B36 : 0xEEE8D5
        case "card", "sky": hex = dark ? 0x073642 : 0xFDF6E3
        case "separator": hex = dark ? 0x113842 : 0xD9D8C7
        case "border", "raised": hex = dark ? 0x586E75 : 0xEEE8D5
        case "text": hex = dark ? 0x93A1A1 : 0x073642
        case "muted": hex = dark ? 0x93A1A1 : 0x586E75
        case "accent", "accentFill": hex = dark ? 0x4BA3DF : 0x1D6FA5
        case "idle": hex = dark ? 0x3CB5AB : 0x167C75
        case "done": hex = dark ? 0x9AAA22 : 0x627200
        case "working": hex = dark ? 0xC69A18 : 0x866500
        case "blocked", "warm": hex = dark ? 0xF07870 : 0xC02B29
        case "addition": hex = 0x859900
        case "deletion": hex = 0xDC322F
        case "onPrimary": hex = dark ? 0x002B36 : 0xFDF6E3
        default: hex = dark ? 0x93A1A1 : 0x586E75
        }
        let effectiveAlpha = token == "muted" ? 1 : (!dark && (token == "addition" || token == "deletion") ? 0.12 : alpha)
        return UIColor(hex: hex, alpha: effectiveAlpha)
    }

    static var accent: Color { color("accent") }
    static var accentFill: Color { color("accentFill") }
    static var sky: Color { color("sky") }
    static var background: Color { color("background") }
    static var backgroundGradient: LinearGradient {
        LinearGradient(colors: [background, background], startPoint: .top, endPoint: .bottom)
    }
    static var listBackground: some View { background }
    static var cardBackground: Color { color("card") }
    static var fieldBackground: Color { color("field") }
    static var hairline: Color { color("border") }
    static var separator: Color { color("separator") }
    static var subtleFill: Color { color("raised") }
    static var rowBackground: Color { cardBackground }
    static var selectedFill: Color { subtleFill }
    static var border: Color { hairline }
    static var text: Color { color("text") }
    static var textMuted: Color {
        color("muted")
    }
    static var warm: Color { color("warm") }
    static var statusIdle: Color { color("idle") }
    static var statusDone: Color { color("done") }
    static var statusWorking: Color { color("working") }
    static var statusBlocked: Color { color("blocked") }
    static var diffAdditionBackground: Color {
        color("addition")
    }
    static var diffDeletionBackground: Color {
        color("deletion")
    }
    static var destructive: Color { statusBlocked }
    static var warning: Color { statusWorking }
    static var onPrimary: Color { color("onPrimary") }
    static var quotaHeadsUp: Color { statusWorking }

    /// Hybrid quota color: mint when remaining ≥ 40% and on pace; amber when
    /// ≥ 40% but behind pace or at 15–39%; blocked red below 15%. A full window
    /// is always green. Outer and inner rings share the same semantic color
    /// (`inner` is ignored for hue). Windows without a schedule use the same
    /// percent bands with no heads-up path.
    static func quotaColor(for chip: AgentUsageChip, now: Date = .now, inner: Bool = false) -> Color {
        _ = inner
        let remaining = chip.percentRemaining
        if remaining >= 100 { return successGreen }
        if remaining < 15 { return destructive }
        if remaining < 40 { return warning }
        if chip.expectedPercentRemaining(now: now) != nil, chip.isBehindPace(now: now) {
            return quotaHeadsUp
        }
        return successGreen
    }

    /// Countdown color: amber when behind pace at ≥ 40% remaining; match
    /// amber/red in the low band; otherwise secondary.
    static func quotaCountdownStyle(for chip: AgentUsageChip, now: Date = .now, inner: Bool = false) -> AnyShapeStyle {
        _ = inner
        if chip.percentRemaining < 40 {
            return AnyShapeStyle(quotaColor(for: chip, now: now))
        }
        if chip.isBehindPace(now: now) {
            return AnyShapeStyle(quotaHeadsUp)
        }
        return AnyShapeStyle(.secondary)
    }

    /// Affirming mint used for on-pace / healthy quota chips.
    static var successGreen: Color { statusDone }
}

// MARK: - Typography Tokens

/// Bundled typefaces from `DESIGN.md` §3: Jost for UI and body text, Silkscreen
/// for tiny pixel labels. Jost ships as the unmodified variable font, whose named
/// instances CoreText exposes as `JostRoman-<Weight>`; the default instance keeps
/// the font's own name, `Jost-Regular`.
enum AppTypeface {
    static func jostName(_ weight: Font.Weight) -> String {
        switch weight {
        case .ultraLight: "JostRoman-ExtraLight"
        case .thin: "JostRoman-Thin"
        case .light: "JostRoman-Light"
        case .medium: "JostRoman-Medium"
        case .semibold: "JostRoman-SemiBold"
        case .bold: "JostRoman-Bold"
        case .heavy: "JostRoman-ExtraBold"
        case .black: "JostRoman-Black"
        default: "Jost-Regular"
        }
    }

    static func silkscreenName(bold: Bool) -> String {
        bold ? "Silkscreen-Bold" : "Silkscreen-Regular"
    }

    /// iOS default (Large) point size and weight for each text style, so
    /// `Font.jost(.caption)` lines up with the system style it replaces.
    static func metrics(for style: Font.TextStyle) -> (size: CGFloat, weight: Font.Weight) {
        switch style {
        case .largeTitle: (34, .regular)
        case .title: (28, .regular)
        case .title2: (22, .regular)
        case .title3: (20, .regular)
        case .headline: (17, .semibold)
        case .callout: (16, .regular)
        case .subheadline: (15, .regular)
        case .footnote: (13, .regular)
        case .caption: (12, .regular)
        case .caption2: (11, .regular)
        default: (17, .regular)
        }
    }

    /// UIKit Jost for navigation bars and segmented controls.
    static func uiJost(_ size: CGFloat, weight: Font.Weight = .regular) -> UIFont {
        UIFont(name: jostName(weight), size: size) ?? .systemFont(ofSize: size)
    }

    /// SwiftUI draws navigation titles, tab labels, and segmented pickers with
    /// UIKit, so they take Jost from appearance proxies rather than `.font`.
    static func installUIKitAppearance() {
        func scaled(_ size: CGFloat, _ weight: Font.Weight, _ style: UIFont.TextStyle) -> UIFont {
            UIFontMetrics(forTextStyle: style).scaledFont(for: uiJost(size, weight: weight))
        }
        let navigationBar = UINavigationBar.appearance()
        navigationBar.titleTextAttributes = [.font: scaled(17, .semibold, .headline)]
        navigationBar.largeTitleTextAttributes = [.font: scaled(34, .semibold, .largeTitle)]

        UITabBarItem.appearance().setTitleTextAttributes([.font: uiJost(10, weight: .medium)], for: .normal)

        let segmented = UISegmentedControl.appearance()
        segmented.setTitleTextAttributes([.font: scaled(13, .regular, .footnote)], for: .normal)
        segmented.setTitleTextAttributes([.font: scaled(13, .semibold, .footnote)], for: .selected)
    }
}

extension Font {
    /// Jost at a fixed point size, replacing `.system(size:weight:)`.
    static func jost(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .custom(AppTypeface.jostName(weight), fixedSize: size)
    }

    /// Jost sized like a system text style and scaled with Dynamic Type.
    static func jost(_ style: Font.TextStyle, weight: Font.Weight? = nil) -> Font {
        let metrics = AppTypeface.metrics(for: style)
        return .custom(AppTypeface.jostName(weight ?? metrics.weight), size: metrics.size, relativeTo: style)
    }

    /// Silkscreen pixel label. Use only at 10 or 12pt for tags and counts, never paragraphs.
    static func pixel(_ size: CGFloat, bold: Bool = false) -> Font {
        .custom(AppTypeface.silkscreenName(bold: bold), fixedSize: size)
    }

    /// 27pt Bold Jost — connection screen hero ("HerdrCat").
    static let appDisplay = Font.jost(27, weight: .bold)

    /// 17pt Semibold Jost — workspace card titles, modal navigation bars.
    static let appHeadlineMd = Font.jost(17, weight: .semibold)

    /// 16pt Semibold Jost — agent card titles, quota card headers.
    static let appHeadlineSm = Font.jost(16, weight: .semibold)

    /// 10pt Silkscreen — agent badge labels, stat count tags.
    static let appLabelSm = Font.pixel(10)
}

// MARK: - Shapes

extension RoundedRectangle {
    /// Convenience initializer ensuring continuous Apple corner smoothing.
    static func continuous(_ cornerRadius: CGFloat) -> RoundedRectangle {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
    }
}

// MARK: - Card View Modifiers

/// Standardizes card surfaces (workspace cards, agent triage cards) with continuous rounding,
/// a stable fill, and a 1pt `--border` outline, as on the website. Cards Herdr reports as
/// focused carry the Herdr focus line.
struct HerdrCardModifier: ViewModifier {
    var isFocused: Bool = false
    var cornerRadius: CGFloat = DesignSystem.CornerRadius.xl
    var showsBorder: Bool = true

    func body(content: Content) -> some View {
        let shape = RoundedRectangle.continuous(cornerRadius)
        return content
            .background(shape.fill(Theme.cardBackground))
            .overlay {
                if showsBorder {
                    shape.strokeBorder(Theme.hairline, lineWidth: 1).allowsHitTesting(false)
                }
            }
            .herdrFocusLine(isFocused, in: shape)
            .contentShape(shape)
    }
}

/// Marks what Herdr reports as focused on the Mac: an accent line on the bottom
/// edge that follows the view's shape up into the corners, tapering as it goes.
/// It is the shape minus a copy raised by `thickness`, like a CSS bottom border
/// on a rounded box. Selection on the phone uses `Theme.selectedFill` instead,
/// so the two states can appear together.
struct HerdrFocusLineModifier<S: Shape>: ViewModifier {
    var isFocused: Bool
    var shape: S
    var thickness: CGFloat = 3

    func body(content: Content) -> some View {
        content.overlay {
            if isFocused {
                shape.subtracting(shape.offset(y: -thickness))
                    .fill(Theme.accent)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
        }
    }
}

/// Standardizes input fields with inset fieldBackground and continuous rounding.
struct HerdrFieldModifier: ViewModifier {
    var cornerRadius: CGFloat = DesignSystem.CornerRadius.md

    func body(content: Content) -> some View {
        let shape = RoundedRectangle.continuous(cornerRadius)
        content
            .background(Theme.fieldBackground, in: shape)
            .contentShape(shape)
    }
}

extension View {
    /// Adds the Herdr focus line along the bottom edge when `isFocused`.
    func herdrFocusLine<S: Shape>(_ isFocused: Bool, in shape: S, thickness: CGFloat = 3) -> some View {
        modifier(HerdrFocusLineModifier(isFocused: isFocused, shape: shape, thickness: thickness))
    }

    /// Applies Herdr card styling (16pt continuous rounding, border, and Herdr focus line).
    func herdrCard(
        isFocused: Bool = false,
        cornerRadius: CGFloat = DesignSystem.CornerRadius.xl,
        showsBorder: Bool = true
    ) -> some View {
        modifier(HerdrCardModifier(isFocused: isFocused, cornerRadius: cornerRadius, showsBorder: showsBorder))
    }

    /// Applies Herdr input field styling (12pt continuous rounding, borderless field background).
    func herdrField(cornerRadius: CGFloat = DesignSystem.CornerRadius.md) -> some View {
        modifier(HerdrFieldModifier(cornerRadius: cornerRadius))
    }
}

// MARK: - Button Styles

/// Primary button style according to DESIGN.md: 8pt rounded rectangle with a solid
/// `accentFill` and `onPrimary` label, bold 14pt typography, 12pt vertical padding,
/// and 44pt minimum height. Pressing shifts it down 1pt (pixel "press"), no shadow.
struct HerdrPrimaryButtonStyle: ButtonStyle {
    var fullWidth: Bool = true
    var cornerRadius: CGFloat = DesignSystem.CornerRadius.button
    var tint: Color? = nil

    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.controlSize) private var controlSize

    private var effectivePaddingVertical: CGFloat {
        switch controlSize {
        case .mini: 4
        case .small: 6
        case .regular, .large, .extraLarge: DesignSystem.Spacing.md
        @unknown default: DesignSystem.Spacing.md
        }
    }

    private var effectivePaddingHorizontal: CGFloat {
        switch controlSize {
        case .mini: 8
        case .small: 12
        case .regular, .large, .extraLarge: DesignSystem.Spacing.base
        @unknown default: DesignSystem.Spacing.base
        }
    }

    private var minHeight: CGFloat {
        switch controlSize {
        case .mini: 24
        case .small: 32
        case .regular, .large, .extraLarge: 44
        @unknown default: 44
        }
    }

    private var effectiveFont: Font {
        switch controlSize {
        case .mini: .jost(11, weight: .bold)
        case .small: .jost(12, weight: .semibold)
        case .regular, .large, .extraLarge: .jost(14, weight: .bold)
        @unknown default: .jost(14, weight: .bold)
        }
    }

    private var shape: RoundedRectangle {
        RoundedRectangle.continuous(cornerRadius)
    }

    @ViewBuilder
    private var backgroundFill: some View {
        if !isEnabled {
            shape.fill(Theme.border)
        } else if let tint {
            shape.fill(tint)
        } else {
            shape.fill(Theme.accentFill)
        }
    }

    private var foregroundColor: Color {
        if !isEnabled {
            return Theme.textMuted
        }
        return Theme.onPrimary
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(effectiveFont)
            .foregroundStyle(foregroundColor)
            .padding(.vertical, effectivePaddingVertical)
            .padding(.horizontal, effectivePaddingHorizontal)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .frame(minHeight: minHeight)
            .background(backgroundFill)
            .contentShape(shape)
            .offset(y: configuration.isPressed ? 1 : 0)
            .opacity(isEnabled ? 1.0 : 0.6)
    }
}

/// Secondary button style according to DESIGN.md: 8pt rounded rectangle with a
/// transparent fill, 2pt `accent` outline and `accent` label, 14pt semibold typography,
/// and 44pt minimum height. Pressing fills it with `--raised` and shifts it down 1pt.
struct HerdrSecondaryButtonStyle: ButtonStyle {
    var fullWidth: Bool = true
    var cornerRadius: CGFloat = DesignSystem.CornerRadius.button

    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.controlSize) private var controlSize

    private var effectivePaddingVertical: CGFloat {
        switch controlSize {
        case .mini: 4
        case .small: 6
        case .regular, .large, .extraLarge: DesignSystem.Spacing.md
        @unknown default: DesignSystem.Spacing.md
        }
    }

    private var effectivePaddingHorizontal: CGFloat {
        switch controlSize {
        case .mini: 8
        case .small: 12
        case .regular, .large, .extraLarge: DesignSystem.Spacing.base
        @unknown default: DesignSystem.Spacing.base
        }
    }

    private var minHeight: CGFloat {
        switch controlSize {
        case .mini: 24
        case .small: 32
        case .regular, .large, .extraLarge: 44
        @unknown default: 44
        }
    }

    private var effectiveFont: Font {
        switch controlSize {
        case .mini: .jost(11, weight: .semibold)
        case .small: .jost(12, weight: .semibold)
        case .regular, .large, .extraLarge: .jost(14, weight: .semibold)
        @unknown default: .jost(14, weight: .semibold)
        }
    }

    private var shape: RoundedRectangle {
        RoundedRectangle.continuous(cornerRadius)
    }

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(effectiveFont)
            .foregroundStyle(isEnabled ? Theme.accent : Theme.textMuted)
            .padding(.vertical, effectivePaddingVertical)
            .padding(.horizontal, effectivePaddingHorizontal)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .frame(minHeight: minHeight)
            .background(shape.fill(configuration.isPressed ? Theme.subtleFill : .clear))
            .overlay(shape.strokeBorder(isEnabled ? Theme.accent : Theme.border, lineWidth: 2))
            .contentShape(shape)
            .offset(y: configuration.isPressed ? 1 : 0)
            .opacity(isEnabled ? 1.0 : 0.45)
    }
}

/// Ghost / borderless button style for secondary dismissive actions (e.g. Cancel):
/// Text-only button with secondary foreground style and 44pt touch target.
struct HerdrGhostButtonStyle: ButtonStyle {
    var fullWidth: Bool = true

    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.jost(14, weight: .medium))
            .foregroundStyle(Theme.textMuted)
            .padding(.vertical, DesignSystem.Spacing.md)
            .padding(.horizontal, DesignSystem.Spacing.base)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .frame(minHeight: 44)
            .contentShape(Rectangle())
            .scaleEffect(!reduceMotion && configuration.isPressed ? 0.98 : 1.0)
            .opacity(configuration.isPressed ? 0.6 : (isEnabled ? 1.0 : 0.4))
            .animation(
                reduceMotion ? nil : .spring(response: 0.25, dampingFraction: 0.7),
                value: configuration.isPressed
            )
    }
}

/// Card button style for interactive cards and rows (recent connections, worktrees, tabs):
/// Provides gentle spring press feedback without plain button highlighting.
struct HerdrCardButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(!reduceMotion && configuration.isPressed ? 0.985 : 1.0)
            .opacity(configuration.isPressed ? 0.88 : 1.0)
            .animation(
                reduceMotion ? nil : .spring(response: 0.25, dampingFraction: 0.7),
                value: configuration.isPressed
            )
    }
}

// MARK: - ButtonStyle Convenience Extensions

extension ButtonStyle where Self == HerdrPrimaryButtonStyle {
    static var herdrPrimary: HerdrPrimaryButtonStyle { .init() }

    static func herdrPrimary(
        fullWidth: Bool = true,
        cornerRadius: CGFloat = DesignSystem.CornerRadius.button,
        tint: Color? = nil
    ) -> HerdrPrimaryButtonStyle {
        HerdrPrimaryButtonStyle(
            fullWidth: fullWidth,
            cornerRadius: cornerRadius,
            tint: tint
        )
    }
}

extension ButtonStyle where Self == HerdrSecondaryButtonStyle {
    static var herdrSecondary: HerdrSecondaryButtonStyle { .init() }

    static func herdrSecondary(
        fullWidth: Bool = true,
        cornerRadius: CGFloat = DesignSystem.CornerRadius.button
    ) -> HerdrSecondaryButtonStyle {
        HerdrSecondaryButtonStyle(
            fullWidth: fullWidth,
            cornerRadius: cornerRadius
        )
    }
}

extension ButtonStyle where Self == HerdrGhostButtonStyle {
    static var herdrGhost: HerdrGhostButtonStyle { .init() }

    static func herdrGhost(fullWidth: Bool = true) -> HerdrGhostButtonStyle {
        HerdrGhostButtonStyle(fullWidth: fullWidth)
    }
}

extension ButtonStyle where Self == HerdrCardButtonStyle {
    static var herdrCard: HerdrCardButtonStyle { .init() }
}

// MARK: - Previews

#Preview("Standard Buttons") {
    VStack(spacing: 16) {
        Button("Primary") {}
            .buttonStyle(.herdrPrimary())

        Button("Primary (Custom Tint)") {}
            .buttonStyle(.herdrPrimary(tint: Theme.destructive))

        Button("Primary (Disabled)") {}
            .buttonStyle(.herdrPrimary())
            .disabled(true)

        Button("Secondary") {}
            .buttonStyle(.herdrSecondary())

        HStack {
            Button("Small Secondary") {}
                .buttonStyle(.herdrSecondary(fullWidth: false))
                .controlSize(.small)

            Button("Small Primary") {}
                .buttonStyle(.herdrPrimary(fullWidth: false))
                .controlSize(.small)
        }

        Button("Ghost / Cancel") {}
            .buttonStyle(.herdrGhost())
    }
    .padding(20)
    .background(Theme.backgroundGradient)
    .preferredColorScheme(.dark)
}

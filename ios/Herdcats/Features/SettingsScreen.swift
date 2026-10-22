import SwiftUI
import UIKit

// MARK: - Settings screen

struct SettingsScreen: View {
    @Environment(AppModel.self) var appModel
    @Bindable var model: SpacesModel
    @Environment(\.scenePhase) var scenePhase
    @State var settingsVisible = false
    @State var phoneAnswersCapability = false
    @State var phoneAnswersChecking = false
    @State var phoneAnswersSelection: PhoneAnswersSelection?
    @State var paneContentPolicy = PaneContentPersistence.shared
    @State var confirmDisablePaneSaving = false
    @State var confirmClearPaneContent = false
    #if DEBUG
    @State var showingOnboardingReplay = false
    #endif

    #if DEBUG
    private var showsDebugReplay: Bool {
        #if targetEnvironment(simulator)
        !ScreenshotFixtures.enabled
        #else
        true
        #endif
    }
    #endif

    @AppStorage(AppearancePreference.storageKey)
    var appearanceRaw = AppearancePreference.default.rawValue
    @AppStorage(LaunchPreference.storageKey)
    var launchRaw = LaunchPreference.default.rawValue
    @AppStorage(PaneTextSizePreference.storageKey)
    var paneTextSizeRaw = PaneTextSizePreference.default.rawValue
    @AppStorage(PaneScrollBehaviorPreference.storageKey)
    var paneScrollBehaviorRaw = PaneScrollBehaviorPreference.default.rawValue
    @AppStorage(PaneComposePreference.storageKey)
    var showComposeByDefault = PaneComposePreference.default
    @AppStorage(DoneClearPreference.storageKey)
    var doneClearRaw = DoneClearPreference.default.rawValue
    @AppStorage(AgentAlertSound.doneStorageKey)
    var doneSoundRaw = AgentAlertSound.defaultDone.rawValue
    @AppStorage(AgentAlertSound.blockedStorageKey)
    var blockedSoundRaw = AgentAlertSound.defaultBlocked.rawValue
    @AppStorage(HapticPreference.storageKey)
    var hapticsEnabled = HapticPreference.default

    var hapticBinding: Binding<Bool> {
        Binding(
            get: { hapticsEnabled },
            set: { enabled in
                hapticsEnabled = enabled
                if enabled {
                    HapticFeedback.selection()
                }
            }
        )
    }

    var paneContentSavingBinding: Binding<Bool> {
        Binding(
            get: { paneContentPolicy.isEnabled },
            set: { enabled in
                if enabled {
                    paneContentPolicy.setEnabled(true)
                } else {
                    confirmDisablePaneSaving = true
                }
            }
        )
    }

    var doneSound: Binding<AgentAlertSound> {
        soundBinding($doneSoundRaw, fallback: .defaultDone)
    }

    var blockedSound: Binding<AgentAlertSound> {
        soundBinding($blockedSoundRaw, fallback: .defaultBlocked)
    }

    /// Previews the tone whenever the user picks a new one.
    func soundBinding(
        _ raw: Binding<String>,
        fallback: AgentAlertSound
    ) -> Binding<AgentAlertSound> {
        Binding(
            get: { AgentAlertSound(rawValue: raw.wrappedValue) ?? fallback },
            set: { sound in
                raw.wrappedValue = sound.rawValue
                sound.play()
            }
        )
    }

    var appearance: Binding<AppearancePreference> {
        Binding(
            get: { AppearancePreference(rawValue: appearanceRaw) ?? .default },
            set: { appearanceRaw = $0.rawValue }
        )
    }

    var launch: Binding<LaunchPreference> {
        Binding(
            get: { LaunchPreference(rawValue: launchRaw) ?? .default },
            set: { launchRaw = $0.rawValue }
        )
    }

    var paneTextSize: Binding<PaneTextSizePreference> {
        Binding(
            get: { PaneTextSizePreference(rawValue: paneTextSizeRaw) ?? .default },
            set: { paneTextSizeRaw = $0.rawValue }
        )
    }

    var paneScrollBehavior: Binding<PaneScrollBehaviorPreference> {
        Binding(
            get: { PaneScrollBehaviorPreference(rawValue: paneScrollBehaviorRaw) ?? .default },
            set: { paneScrollBehaviorRaw = $0.rawValue }
        )
    }

    var doneClear: Binding<DoneClearPreference> {
        Binding(
            get: { DoneClearPreference(rawValue: doneClearRaw) ?? .default },
            set: { doneClearRaw = $0.rawValue }
        )
    }

    /// App version from the bundle, e.g. "0.1.0 (1)".
    static var appVersion: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
        if version.isEmpty { return build }
        if build.isEmpty { return version }
        return "\(version) (\(build))"
    }

    var body: some View {
        NavigationStack {
            Form {
                #if DEBUG
                if showsDebugReplay {
                    OnboardingReplaySection {
                        showingOnboardingReplay = true
                    }
                }
                #endif

                refreshSection
                panePrivacySection
                themeSection
                paneSection

                soundSection
                launchSection
                connectionSection
                phoneAnswersSection
                versionSection

            }
            .scrollContentBackground(.hidden)
            .background(Theme.listBackground.ignoresSafeArea())
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    HerdrcatBrandMark()
                }
            }
            .toolbarBackground(.hidden, for: .navigationBar)
            .confirmationDialog(
                "Stop Preserving Prompts and History?",
                isPresented: $confirmDisablePaneSaving,
                titleVisibility: .visible
            ) {
                Button("Stop Preserving and Erase Data", role: .destructive) {
                    paneContentPolicy.setEnabled(false)
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(
                    "All saved drafts and command history on this iPhone will be deleted. Moving between panes will no longer restore uncommitted text."
                )
            }
            .confirmationDialog(
                "Clear Saved Drafts and History?",
                isPresented: $confirmClearPaneContent,
                titleVisibility: .visible
            ) {
                Button("Clear Saved Data", role: .destructive) {
                    paneContentPolicy.eraseSavedContent()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(
                    "Erases all saved drafts and command history across all remote machines from this iPhone. Remote terminal panes are not affected."
                )
            }
            .onAppear { settingsVisible = true }
            .onDisappear { settingsVisible = false }
            .task(id: phoneAnswersScreenKey) { await preparePhoneAnswersSettings() }
            .sheet(item: $phoneAnswersSelection) { selection in
                PhoneAnswersSheet(selection: selection, connection: appModel.connection, store: appModel.phoneAnswers,
                    isCurrent: { phoneAnswersContextIsCurrent(selection.key) })
            }
            .connectionStatusBanner(appModel: appModel)
            #if DEBUG
            .fullScreenCover(isPresented: $showingOnboardingReplay) {
                DebugOnboardingReplayView()
            }
            #endif
        }
    }
}

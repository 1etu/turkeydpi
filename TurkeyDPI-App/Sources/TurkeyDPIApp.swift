import SwiftUI
import AppKit

@main
struct TurkeyDPIApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @StateObject private var appState = AppState()
    
    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(appState)
                .frame(minWidth: 800, minHeight: 500)
                .task {
                    await appState.startAutoStartContainers()
                }
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Container") {
                    appState.showingCreateSheet = true
                }
                .keyboardShortcut("n", modifiers: .command)
            }
        }
        
        MenuBarExtra("TurkeyDPI", systemImage: appState.hasRunningContainers ? "circle.fill" : "circle") {
            MenuBarView()
                .environmentObject(appState)
        }
        .menuBarExtraStyle(.window)
    }
}

class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        recoverStrandedProxy()

        NSApplication.shared.setActivationPolicy(.regular)
        
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            NSApplication.shared.activate(ignoringOtherApps: true)
            
            for window in NSApplication.shared.windows {
                if window.canBecomeKey {
                    window.makeKeyAndOrderFront(nil)
                    window.center()
                    break
                }
            }
            
            if NSApplication.shared.windows.filter({ $0.isVisible }).isEmpty {
                NSApp.sendAction(Selector(("newWindowForTab:")), to: nil, from: nil)
            }
        }
    }
    
    func applicationWillTerminate(_ notification: Notification) {
        SystemProxy.disableAllProxiesSync()
        SystemProxy.clearOwnershipFlag()
    }

    private func recoverStrandedProxy() {
        guard SystemProxy.hasOwnershipFlag() else { return }

        SystemProxy.disableAllProxiesSync()
        SystemProxy.clearOwnershipFlag()
    }
    
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return false
    }
    
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            for window in NSApplication.shared.windows {
                if window.canBecomeKey {
                    window.makeKeyAndOrderFront(nil)
                    return true
                }
            }
        }
        return true
    }
}

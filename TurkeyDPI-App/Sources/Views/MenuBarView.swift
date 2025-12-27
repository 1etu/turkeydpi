import SwiftUI

struct MenuBarView: View {
    @EnvironmentObject var appState: AppState
    
    var body: some View {
        VStack(spacing: 0) {
            if appState.containers.isEmpty {
                VStack(spacing: 6) {
                    Text("No proxies configured")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 20)
                .frame(maxWidth: .infinity)
            } else {
                VStack(spacing: 2) {
                    ForEach(appState.containers) { container in
                        MenuBarContainerRow(container: container)
                    }
                }
                .padding(6)
            }
            
            Divider()
            
            HStack {
                Button("Open") {
                    NSApp.activate(ignoringOtherApps: true)
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                
                Spacer()
                
                Button("Quit") {
                    Task {
                        for container in appState.containers where container.status == .running {
                            await appState.stopContainer(container)
                        }
                        NSApp.terminate(nil)
                    }
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .frame(width: 200)
    }
}

struct MenuBarContainerRow: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var container: ProxyContainer
    
    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(container.status == .running ? Color.green : Color(nsColor: .tertiaryLabelColor))
                .frame(width: 6, height: 6)
            
            Text(container.config.name)
                .font(.system(size: 11))
                .lineLimit(1)
            
            Spacer()
            
            Button(action: { toggleContainer() }) {
                Text(container.status == .running ? "Stop" : "Start")
                    .font(.system(size: 10))
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.primary.opacity(0.03))
        .cornerRadius(4)
    }
    
    private func toggleContainer() {
        Task {
            if container.status == .running {
                await appState.stopContainer(container)
            } else {
                await appState.startContainer(container)
            }
        }
    }
}

#Preview {
    MenuBarView()
        .environmentObject(AppState())
}

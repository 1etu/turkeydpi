import SwiftUI

struct ContentView: View {
    @EnvironmentObject var appState: AppState
    
    var body: some View {
        NavigationSplitView {
            SidebarView()
        } detail: {
            if let container = appState.selectedContainer {
                ContainerDetailView(container: container)
            } else {
                EmptyStateView()
            }
        }
        .sheet(isPresented: $appState.showingCreateSheet) {
            CreateContainerSheet()
        }
    }
}


struct SidebarView: View {
    @EnvironmentObject var appState: AppState
    
    var body: some View {
        VStack(spacing: 0) {
            if appState.containers.isEmpty {
                VStack(spacing: 10) {
                    Spacer()
                    Text("No Proxies")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text("Click + to create one")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                    Spacer()
                }
                .frame(maxWidth: .infinity)
            } else {
                List(appState.containers, selection: $appState.selectedContainerId) { container in
                    ContainerRow(container: container)
                        .tag(container.id)
                        .contextMenu {
                            ContainerContextMenu(container: container)
                        }
                }
                .listStyle(.sidebar)
            }
        }
        .frame(minWidth: 200)
        .toolbar {
            ToolbarItem(placement: .automatic) {
                Button(action: { appState.showingCreateSheet = true }) {
                    Image(systemName: "plus")
                }
                .help("New Proxy")
            }
        }
    }
}

struct ContainerRow: View {
    @ObservedObject var container: ProxyContainer
    
    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(statusColor)
                .frame(width: 7, height: 7)
            
            VStack(alignment: .leading, spacing: 1) {
                Text(container.config.name)
                    .font(.system(size: 12))
                    .lineLimit(1)
                
                Text(":\(container.config.listenPort)")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            
            Spacer()
        }
        .padding(.vertical, 2)
    }
    
    private var statusColor: Color {
        switch container.status {
        case .running: return .green
        case .stopped, .error: return Color(nsColor: .tertiaryLabelColor)
        case .starting, .stopping: return .orange
        }
    }
}

struct ContainerContextMenu: View {
    @EnvironmentObject var appState: AppState
    let container: ProxyContainer
    
    var body: some View {
        Group {
            if container.status == .running {
                Button("Stop") {
                    Task { await appState.stopContainer(container) }
                }
            } else {
                Button("Start") {
                    Task { await appState.startContainer(container) }
                }
            }
            
            Divider()
            
            Button("Clear Logs") {
                container.clearLogs()
            }
            
            Divider()
            
            Button("Delete", role: .destructive) {
                appState.deleteContainer(container)
            }
        }
    }
}


struct EmptyStateView: View {
    @EnvironmentObject var appState: AppState
    
    var body: some View {
        VStack(spacing: 12) {
            Text("No Proxy Selected")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
            
            if appState.containers.isEmpty {
                Button("Create Proxy") {
                    appState.showingCreateSheet = true
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(NSColor.textBackgroundColor))
    }
}

#Preview {
    ContentView()
        .environmentObject(AppState())
        .frame(width: 900, height: 600)
}

import SwiftUI


struct ConnectionEvent: Identifiable {
    let id = UUID()
    let timestamp: Date
    let domain: String
    let status: ConnectionStatus
    let rawLog: String
    
    enum ConnectionStatus {
        case fragmented
        case connected
        case failed
        case unknown
    }
    
    var faviconURL: URL? {
        URL(string: "https://www.google.com/s2/favicons?sz=32&domain=\(domain)")
    }
    
    var shortDomain: String {
        let parts = domain.split(separator: ".")
        if parts.count >= 2 {
            return parts.suffix(2).joined(separator: ".")
        }
        return domain
    }
}


struct ContainerDetailView: View {
    @EnvironmentObject var appState: AppState
    @ObservedObject var container: ProxyContainer
    @State private var viewMode: ViewMode = .connections
    @State private var watchedDomain: String? = nil
    
    enum ViewMode: String, CaseIterable {
        case connections = "Connections"
        case raw = "Raw Logs"
    }
    
    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            
            if viewMode == .connections {
                ConnectionsView(container: container, watchedDomain: $watchedDomain)
            } else {
                RawLogView(container: container)
            }
        }
        .background(Color(NSColor.textBackgroundColor))
    }
    
    private var toolbar: some View {
        HStack(spacing: 12) {
            HStack(spacing: 6) {
                Circle()
                    .fill(container.status == .running ? Color.green : Color(nsColor: .tertiaryLabelColor))
                    .frame(width: 8, height: 8)
                
                Text(container.config.name)
                    .font(.system(size: 13, weight: .medium))
            }
            
            Text("•")
                .foregroundStyle(.quaternary)
            
            Text(":\(container.config.listenPort)")
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
            
            Spacer()
            
            Picker("", selection: $viewMode) {
                ForEach(ViewMode.allCases, id: \.self) { mode in
                    Text(mode.rawValue).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .frame(width: 180)
            
            HStack(spacing: 8) {
                Button(action: { container.clearLogs() }) {
                    Image(systemName: "trash")
                        .font(.system(size: 11))
                }
                .buttonStyle(.borderless)
                .help("Clear")
                
                if container.status == .running {
                    Button("Stop") { Task { await appState.stopContainer(container) } }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                } else if container.status == .starting || container.status == .stopping {
                    ProgressView().scaleEffect(0.6).frame(width: 50)
                } else {
                    Button("Start") { Task { await appState.startContainer(container) } }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Color(NSColor.windowBackgroundColor))
    }
}


struct ConnectionsView: View {
    @ObservedObject var container: ProxyContainer
    @Binding var watchedDomain: String?
    @State private var searchText = ""
    
    private var connections: [ConnectionEvent] {
        container.logs.compactMap { entry -> ConnectionEvent? in
            guard let domain = extractDomain(from: entry.message) else { return nil }
            
            let status: ConnectionEvent.ConnectionStatus
            if entry.message.contains("fragmented") || entry.message.contains("SNI fragmented") {
                status = .fragmented
            } else if entry.message.contains("connected") || entry.message.contains("established") {
                status = .connected
            } else if entry.message.contains("error") || entry.message.contains("failed") {
                status = .failed
            } else {
                status = .unknown
            }
            
            return ConnectionEvent(
                timestamp: entry.timestamp,
                domain: domain,
                status: status,
                rawLog: entry.message
            )
        }
    }
    
    private var filteredConnections: [ConnectionEvent] {
        connections.filter { conn in
            let matchesSearch = searchText.isEmpty || conn.domain.localizedCaseInsensitiveContains(searchText)
            let matchesWatch = watchedDomain == nil || conn.domain.contains(watchedDomain!)
            return matchesSearch && matchesWatch
        }
    }
    
    private var domainStats: [(domain: String, count: Int)] {
        var counts: [String: Int] = [:]
        for conn in connections {
            counts[conn.shortDomain, default: 0] += 1
        }
        return counts.sorted { $0.value > $1.value }.prefix(10).map { ($0.key, $0.value) }
    }
    
    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    HStack(spacing: 4) {
                        Image(systemName: "magnifyingglass")
                            .font(.system(size: 10))
                            .foregroundStyle(.tertiary)
                        TextField("Search domains...", text: $searchText)
                            .textFieldStyle(.plain)
                            .font(.system(size: 11))
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .cornerRadius(6)
                    
                    if let watched = watchedDomain {
                        HStack(spacing: 4) {
                            Image(systemName: "eye.fill")
                                .font(.system(size: 9))
                            Text(watched)
                                .font(.system(size: 10))
                            Button(action: { watchedDomain = nil }) {
                                Image(systemName: "xmark")
                                    .font(.system(size: 8, weight: .bold))
                            }
                            .buttonStyle(.plain)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color.accentColor.opacity(0.15))
                        .foregroundColor(.accentColor)
                        .cornerRadius(4)
                    }
                    
                    Spacer()
                    
                    Text("\(filteredConnections.count) connections")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                }
                .padding(10)
                .background(Color(NSColor.windowBackgroundColor).opacity(0.5))
                
                Divider()
                
                if filteredConnections.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "network.slash")
                            .font(.system(size: 24))
                            .foregroundStyle(.quaternary)
                        Text("No connections yet")
                            .font(.system(size: 11))
                            .foregroundStyle(.tertiary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(spacing: 1) {
                            ForEach(filteredConnections.reversed()) { conn in
                                ConnectionRow(connection: conn, onWatch: { watchedDomain = $0 })
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }
            }
            .frame(minWidth: 400)
            
            VStack(spacing: 0) {
                HStack {
                    Text("Top Domains")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                }
                .padding(10)
                .background(Color(NSColor.windowBackgroundColor).opacity(0.5))
                
                Divider()
                
                if domainStats.isEmpty {
                    Text("No data")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        VStack(spacing: 2) {
                            ForEach(domainStats, id: \.domain) { stat in
                                DomainStatRow(
                                    domain: stat.domain,
                                    count: stat.count,
                                    isWatched: watchedDomain == stat.domain,
                                    onTap: {
                                        if watchedDomain == stat.domain {
                                            watchedDomain = nil
                                        } else {
                                            watchedDomain = stat.domain
                                        }
                                    }
                                )
                            }
                        }
                        .padding(8)
                    }
                }
            }
            .frame(width: 180)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.3))
        }
    }
    
    private func extractDomain(from message: String) -> String? {
        let ansiPattern = #"\x1B\[[0-9;]*[a-zA-Z]"#
        let cleanMessage: String
        if let ansiRegex = try? NSRegularExpression(pattern: ansiPattern, options: []) {
            cleanMessage = ansiRegex.stringByReplacingMatches(
                in: message,
                options: [],
                range: NSRange(message.startIndex..., in: message),
                withTemplate: ""
            )
        } else {
            cleanMessage = message
        }
        
        let bracketPattern = #"\[\d+m"#
        let finalMessage: String
        if let bracketRegex = try? NSRegularExpression(pattern: bracketPattern, options: []) {
            finalMessage = bracketRegex.stringByReplacingMatches(
                in: cleanMessage,
                options: [],
                range: NSRange(cleanMessage.startIndex..., in: cleanMessage),
                withTemplate: ""
            )
        } else {
            finalMessage = cleanMessage
        }
        
        let domainPattern = #"([a-zA-Z0-9]([a-zA-Z0-9\-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}"#
        
        if let regex = try? NSRegularExpression(pattern: domainPattern, options: []) {
            let matches = regex.matches(in: finalMessage, options: [], range: NSRange(finalMessage.startIndex..., in: finalMessage))
            
            for match in matches {
                if let range = Range(match.range, in: finalMessage) {
                    let domain = String(finalMessage[range])
                    if !domain.hasSuffix(".rs") && 
                       !domain.contains("::") &&
                       !domain.hasPrefix("backend") &&
                       !domain.hasPrefix("engine") &&
                       !domain.hasPrefix("transparent") &&
                       domain.count > 4 {
                        return domain
                    }
                }
            }
        }
        return nil
    }
}

struct ConnectionRow: View {
    let connection: ConnectionEvent
    let onWatch: (String) -> Void
    @State private var isHovered = false
    
    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f
    }()
    
    var body: some View {
        HStack(spacing: 10) {
            Circle()
                .fill(statusColor)
                .frame(width: 6, height: 6)
            
            AsyncImage(url: connection.faviconURL) { image in
                image.resizable()
            } placeholder: {
                Image(systemName: "globe")
                    .foregroundStyle(.tertiary)
            }
            .frame(width: 16, height: 16)
            .cornerRadius(2)
            
            VStack(alignment: .leading, spacing: 1) {
                Text(connection.domain)
                    .font(.system(size: 11, weight: .medium))
                    .lineLimit(1)
                
                Text(statusLabel)
                    .font(.system(size: 9))
                    .foregroundStyle(statusColor)
            }
            
            Spacer()
            
            Text(Self.timeFormatter.string(from: connection.timestamp))
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.tertiary)
            
            if isHovered {
                Button(action: { onWatch(connection.shortDomain) }) {
                    Image(systemName: "eye")
                        .font(.system(size: 10))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("Watch this domain")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(isHovered ? Color.primary.opacity(0.04) : .clear)
        .onHover { isHovered = $0 }
    }
    
    private var statusColor: Color {
        switch connection.status {
        case .fragmented: return .green
        case .connected: return .blue
        case .failed: return .red
        case .unknown: return .secondary
        }
    }
    
    private var statusLabel: String {
        switch connection.status {
        case .fragmented: return "SNI Fragmented"
        case .connected: return "Connected"
        case .failed: return "Failed"
        case .unknown: return "Request"
        }
    }
}

struct DomainStatRow: View {
    let domain: String
    let count: Int
    let isWatched: Bool
    let onTap: () -> Void
    @State private var isHovered = false
    
    var body: some View {
        Button(action: onTap) {
            HStack(spacing: 6) {
                AsyncImage(url: URL(string: "https://www.google.com/s2/favicons?sz=16&domain=\(domain)")) { image in
                    image.resizable()
                } placeholder: {
                    Image(systemName: "globe")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                }
                .frame(width: 12, height: 12)
                
                Text(domain)
                    .font(.system(size: 10))
                    .lineLimit(1)
                    .foregroundColor(isWatched ? .accentColor : .primary)
                
                Spacer()
                
                Text("\(count)")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Color.primary.opacity(0.06))
                    .cornerRadius(3)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(isWatched ? Color.accentColor.opacity(0.1) : (isHovered ? Color.primary.opacity(0.04) : .clear))
            .cornerRadius(4)
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
    }
}


struct RawLogView: View {
    @ObservedObject var container: ProxyContainer
    @State private var autoScroll = true
    @State private var searchText = ""
    
    private var filteredLogs: [LogEntry] {
        if searchText.isEmpty {
            return container.logs
        }
        return container.logs.filter { $0.message.localizedCaseInsensitiveContains(searchText) }
    }
    
    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                HStack(spacing: 4) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                    TextField("Filter logs...", text: $searchText)
                        .textFieldStyle(.plain)
                        .font(.system(size: 11))
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(Color(nsColor: .controlBackgroundColor))
                .cornerRadius(6)
                .frame(width: 200)
                
                Spacer()
                
                Toggle(isOn: $autoScroll) {
                    Image(systemName: "arrow.down.to.line")
                        .font(.system(size: 10))
                }
                .toggleStyle(.button)
                .controlSize(.small)
                
                Text("\(filteredLogs.count)")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.tertiary)
            }
            .padding(10)
            .background(Color(NSColor.windowBackgroundColor).opacity(0.5))
            
            Divider()
            
            if filteredLogs.isEmpty {
                Text("No logs")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(filteredLogs) { entry in
                                RawLogRow(entry: entry)
                                    .id(entry.id)
                            }
                        }
                    }
                    .onChange(of: container.logs.count) { _, _ in
                        if autoScroll, let last = filteredLogs.last {
                            withAnimation(.easeOut(duration: 0.1)) {
                                proxy.scrollTo(last.id, anchor: .bottom)
                            }
                        }
                    }
                }
            }
        }
    }
}

struct RawLogRow: View {
    let entry: LogEntry
    @State private var isHovered = false
    
    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss.SSS"
        return f
    }()
    
    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            Text(Self.timeFormatter.string(from: entry.timestamp))
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.tertiary)
                .frame(width: 80, alignment: .leading)
            
            Text(entry.type.shortLabel)
                .font(.system(size: 9, weight: .medium, design: .monospaced))
                .foregroundStyle(entry.type.labelColor)
                .frame(width: 28, alignment: .leading)
            
            Text(entry.message)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(entry.type == .error ? .red : .primary)
                .textSelection(.enabled)
            
            Spacer()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 2)
        .background(isHovered ? Color.primary.opacity(0.03) : .clear)
        .onHover { isHovered = $0 }
    }
}

extension LogEntry.LogType {
    var shortLabel: String {
        switch self {
        case .info: return "INF"
        case .success: return "OK"
        case .warning: return "WRN"
        case .error: return "ERR"
        case .debug: return "DBG"
        }
    }
    
    var labelColor: Color {
        switch self {
        case .info: return .secondary
        case .success: return .green
        case .warning: return .orange
        case .error: return .red
        case .debug: return .purple
        }
    }
}

#Preview {
    let state = AppState()
    let config = ContainerConfig(name: "Test", listenPort: 8844)
    state.createContainer(config: config)
    
    return ContainerDetailView(container: state.containers.first!)
        .environmentObject(state)
        .frame(width: 800, height: 500)
}

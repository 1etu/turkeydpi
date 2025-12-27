import SwiftUI
import Combine

@MainActor
class AppState: ObservableObject {
    @Published var containers: [ProxyContainer] = []
    @Published var selectedContainerId: UUID?
    @Published var showingCreateSheet = false
    
    var selectedContainer: ProxyContainer? {
        containers.first { $0.id == selectedContainerId }
    }
    
    var hasRunningContainers: Bool {
        containers.contains { $0.status == .running }
    }
    
    init() {
        loadContainers()
    }
    
    func createContainer(config: ContainerConfig) {
        let container = ProxyContainer(config: config)
        containers.append(container)
        selectedContainerId = container.id
        saveContainers()
    }
    
    func deleteContainer(_ container: ProxyContainer) {
        Task {
            await container.stop()
        }
        containers.removeAll { $0.id == container.id }
        if selectedContainerId == container.id {
            selectedContainerId = containers.first?.id
        }
        saveContainers()
    }
    
    func startContainer(_ container: ProxyContainer) async {
        await container.start()
        objectWillChange.send()
    }
    
    func stopContainer(_ container: ProxyContainer) async {
        await container.stop()
        objectWillChange.send()
    }
        
    private func saveContainers() {
        let configs = containers.map { $0.config }
        if let data = try? JSONEncoder().encode(configs) {
            UserDefaults.standard.set(data, forKey: "savedContainers")
        }
    }
    
    private func loadContainers() {
        guard let data = UserDefaults.standard.data(forKey: "savedContainers"),
              let configs = try? JSONDecoder().decode([ContainerConfig].self, from: data) else {
            return
        }
        containers = configs.map { ProxyContainer(config: $0) }
        selectedContainerId = containers.first?.id
    }
}

struct ContainerConfig: Codable, Identifiable {
    var id = UUID()
    var name: String
    var listenAddress: String = "127.0.0.1"
    var listenPort: Int = 8844
    var preset: ISPPreset = .aggressive
    var autoStart: Bool = false
    var enableSystemProxy: Bool = true
    
    var fullAddress: String {
        "\(listenAddress):\(listenPort)"
    }
}

enum ISPPreset: String, Codable, CaseIterable, Identifiable {
    case turkTelekom = "turk-telekom"
    case vodafone = "vodafone"
    case superonline = "superonline"
    case aggressive = "aggressive"
    
    var id: String { rawValue }
    
    var displayName: String {
        switch self {
        case .turkTelekom: return "Türk Telekom"
        case .vodafone: return "Vodafone TR"
        case .superonline: return "Superonline"
        case .aggressive: return "Aggressive"
        }
    }
    
    var description: String {
        switch self {
        case .turkTelekom: return "Split at byte 2"
        case .vodafone: return "Split at byte 3 with delay"
        case .superonline: return "Split at byte 1"
        case .aggressive: return "All techniques enabled"
        }
    }
}

enum ContainerStatus: String {
    case stopped = "Stopped"
    case starting = "Starting"
    case running = "Running"
    case stopping = "Stopping"
    case error = "Error"
    
    var color: Color {
        switch self {
        case .stopped: return .secondary
        case .starting, .stopping: return .orange
        case .running: return .green
        case .error: return .red
        }
    }
    
    var icon: String {
        switch self {
        case .stopped: return "stop.circle"
        case .starting, .stopping: return "clock"
        case .running: return "play.circle.fill"
        case .error: return "exclamationmark.triangle"
        }
    }
}

@MainActor
class ProxyContainer: ObservableObject, Identifiable {
    let id: UUID
    let config: ContainerConfig
    
    @Published var status: ContainerStatus = .stopped
    @Published var logs: [LogEntry] = []
    @Published var connectionCount: Int = 0
    @Published var bytesTransferred: UInt64 = 0
    
    private var process: Process?
    private var outputPipe: Pipe?
    private var errorPipe: Pipe?
    
    init(config: ContainerConfig) {
        self.id = config.id
        self.config = config
    }
    
    func start() async {
        guard status != .running else { return }
        
        status = .starting
        addLog("Starting proxy on \(config.fullAddress)...", type: .info)
        
        guard let binaryPath = findBinary() else {
            status = .error
            addLog("Error: turkeydpi binary not found", type: .error)
            return
        }
        
        if config.enableSystemProxy {
            let enabled = await SystemProxy.enableSOCKSProxy(
                host: config.listenAddress,
                port: String(config.listenPort)
            )
            if enabled {
                addLog("System SOCKS proxy enabled", type: .info)
            } else {
                addLog("Warning: Could not enable system proxy", type: .warning)
            }
        }
        
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binaryPath)
        process.arguments = [
            "bypass",
            "-l", config.fullAddress,
            "--preset", config.preset.rawValue
        ]
        
        let outputPipe = Pipe()
        let errorPipe = Pipe()
        process.standardOutput = outputPipe
        process.standardError = errorPipe
        
        self.process = process
        self.outputPipe = outputPipe
        self.errorPipe = errorPipe
        
        outputPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            if let output = String(data: data, encoding: .utf8), !output.isEmpty {
                Task { @MainActor [weak self] in
                    self?.processOutput(output)
                }
            }
        }
        
        errorPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            if let output = String(data: data, encoding: .utf8), !output.isEmpty {
                Task { @MainActor [weak self] in
                    self?.processOutput(output, isError: true)
                }
            }
        }
        
        process.terminationHandler = { [weak self] proc in
            Task { @MainActor [weak self] in
                self?.handleTermination(exitCode: proc.terminationStatus)
            }
        }
        
        do {
            try process.run()
            status = .running
            addLog("Proxy started successfully", type: .success)
        } catch {
            status = .error
            addLog("Failed to start: \(error.localizedDescription)", type: .error)
        }
    }
    
    func stop() async {
        guard status == .running || status == .starting else { return }
        
        status = .stopping
        addLog("Stopping proxy...", type: .info)
        
        if config.enableSystemProxy {
            let _ = await SystemProxy.disableAllProxies()
            addLog("System proxies disabled", type: .info)
        }
        
        // Force kill the process
        if let proc = process {
            proc.terminate()
            
            // Give it a moment, then force kill if needed
            try? await Task.sleep(nanoseconds: 500_000_000)
            if proc.isRunning {
                kill(proc.processIdentifier, SIGKILL)
            }
        }
        process = nil
        
        outputPipe?.fileHandleForReading.readabilityHandler = nil
        errorPipe?.fileHandleForReading.readabilityHandler = nil
        
        status = .stopped
        addLog("Proxy stopped", type: .info)
    }
    
    func clearLogs() {
        logs.removeAll()
    }
        
    private func findBinary() -> String? {
        let paths = [
            Bundle.main.bundlePath + "/Contents/MacOS/turkeydpi-engine",
            Bundle.main.bundlePath + "/Contents/Resources/turkeydpi-engine",
            Bundle.main.bundlePath + "/Contents/MacOS/turkeydpi",
            Bundle.main.bundlePath + "/Contents/Resources/turkeydpi",
            "/usr/local/bin/turkeydpi",
            "/opt/homebrew/bin/turkeydpi",
            FileManager.default.homeDirectoryForCurrentUser.path + "/.cargo/bin/turkeydpi",
            FileManager.default.homeDirectoryForCurrentUser.path + "/Desktop/turkeydpi/target/release/turkeydpi",
            FileManager.default.homeDirectoryForCurrentUser.path + "/Desktop/turkeydpi/target/debug/turkeydpi"
        ]
        return paths.first { FileManager.default.fileExists(atPath: $0) }
    }
    
    private func processOutput(_ output: String, isError: Bool = false) {
        let lines = output.components(separatedBy: .newlines).filter { !$0.isEmpty }
        for line in lines {
            let type: LogEntry.LogType = isError ? .error : parseLogType(from: line)
            addLog(line, type: type)
            
            if line.contains("New connection") || line.contains("Connection from") {
                connectionCount += 1
            }
        }
    }
    
    private func parseLogType(from line: String) -> LogEntry.LogType {
        if line.contains("ERROR") || line.contains("error") {
            return .error
        } else if line.contains("WARN") || line.contains("warn") {
            return .warning
        } else if line.contains("INFO") || line.contains("info") {
            return .info
        } else if line.contains("DEBUG") || line.contains("TRACE") {
            return .debug
        }
        return .info
    }
    
    private func handleTermination(exitCode: Int32) {
        outputPipe?.fileHandleForReading.readabilityHandler = nil
        errorPipe?.fileHandleForReading.readabilityHandler = nil
        
        if status != .stopping {
            status = exitCode == 0 ? .stopped : .error
            addLog("Process exited with code \(exitCode)", type: exitCode == 0 ? .info : .error)
        }
    }
    
    private func addLog(_ message: String, type: LogEntry.LogType) {
        let entry = LogEntry(message: message, type: type)
        logs.append(entry)
        
        if logs.count > 1000 {
            logs.removeFirst(100)
        }
    }
}

struct LogEntry: Identifiable {
    let id = UUID()
    let timestamp = Date()
    let message: String
    let type: LogType
    
    enum LogType {
        case info, success, warning, error, debug
        
        var color: Color {
            switch self {
            case .info: return .primary
            case .success: return .green
            case .warning: return .orange
            case .error: return .red
            case .debug: return .secondary
            }
        }
        
        var icon: String {
            switch self {
            case .info: return "info.circle"
            case .success: return "checkmark.circle"
            case .warning: return "exclamationmark.triangle"
            case .error: return "xmark.circle"
            case .debug: return "ant"
            }
        }
    }
}

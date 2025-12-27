import Foundation

struct SystemProxy {
    static let proxyHost = "127.0.0.1"
    static let proxyPort = "8844"
    
    static func getActiveNetworkService() -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        process.arguments = ["-listnetworkserviceorder"]
        
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        
        do {
            try process.run()
            process.waitUntilExit()
            
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            guard let output = String(data: data, encoding: .utf8) else { return nil }
            
            let lines = output.components(separatedBy: "\n")
            for line in lines {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.hasPrefix("(") && (trimmed.contains("Wi-Fi") || trimmed.contains("Ethernet")) {
                    if let range = trimmed.range(of: #"\(\d+\)\s+"#, options: .regularExpression) {
                        return String(trimmed[range.upperBound...])
                    }
                }
            }
            
            return "Wi-Fi" // defaullt
        } catch {
            return "Wi-Fi"
        }
    }
    
    static func enableSOCKSProxy(host: String = proxyHost, port: String = proxyPort) async -> Bool {
        guard let service = getActiveNetworkService() else { return false }
        
        var success = await runNetworkSetup(["-setwebproxy", service, host, port])
        guard success else { return false }
        success = await runNetworkSetup(["-setwebproxystate", service, "on"])
        guard success else { return false }
        
        success = await runNetworkSetup(["-setsecurewebproxy", service, host, port])
        guard success else { return false }
        success = await runNetworkSetup(["-setsecurewebproxystate", service, "on"])
        
        return success
    }
    
    static func disableSOCKSProxy() async -> Bool {
        guard let service = getActiveNetworkService() else { return false }
        return await runNetworkSetup(["-setsocksfirewallproxystate", service, "off"])
    }
    
    static func disableAllProxies() async -> Bool {
        guard let service = getActiveNetworkService() else { return false }
        
        var success = await runNetworkSetup(["-setsocksfirewallproxystate", service, "off"])
        let http = await runNetworkSetup(["-setwebproxystate", service, "off"])
        let https = await runNetworkSetup(["-setsecurewebproxystate", service, "off"])
        
        return success && http && https
    }
    
    
    private static func runNetworkSetup(_ arguments: [String]) async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
                process.arguments = arguments
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.nullDevice
                
                do {
                    try process.run()
                    process.waitUntilExit()
                    continuation.resume(returning: process.terminationStatus == 0)
                } catch {
                    continuation.resume(returning: false)
                }
            }
        }
    }
}

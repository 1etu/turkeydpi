import Foundation

struct SystemProxy {
    static let proxyHost = "127.0.0.1"
    static let proxyPort = "8844"

    private static let ownershipKey = "systemProxyOwnedByUs"

    static func hasOwnershipFlag() -> Bool {
        UserDefaults.standard.bool(forKey: ownershipKey)
    }

    static func setOwnershipFlag() {
        UserDefaults.standard.set(true, forKey: ownershipKey)
        UserDefaults.standard.synchronize()
    }

    static func clearOwnershipFlag() {
        UserDefaults.standard.set(false, forKey: ownershipKey)
        UserDefaults.standard.synchronize()
    }

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

            return "Wi-Fi"
        } catch {
            return "Wi-Fi"
        }
    }

    static func enableHTTPProxy(host: String = proxyHost, port: String = proxyPort) async -> Bool {
        guard let service = getActiveNetworkService() else { return false }

        setOwnershipFlag()

        var success = await runNetworkSetup(["-setwebproxy", service, host, port])
        guard success else { return false }
        success = await runNetworkSetup(["-setwebproxystate", service, "on"])
        guard success else { return false }

        success = await runNetworkSetup(["-setsecurewebproxy", service, host, port])
        guard success else { return false }
        success = await runNetworkSetup(["-setsecurewebproxystate", service, "on"])

        return success
    }

    static func disableAllProxies() async -> Bool {
        guard let service = getActiveNetworkService() else { return false }

        let socks = await runNetworkSetup(["-setsocksfirewallproxystate", service, "off"])
        let http = await runNetworkSetup(["-setwebproxystate", service, "off"])
        let https = await runNetworkSetup(["-setsecurewebproxystate", service, "off"])

        clearOwnershipFlag()

        return socks && http && https
    }

    @discardableResult
    static func disableAllProxiesSync() -> Bool {
        guard let service = getActiveNetworkService() else { return false }

        let socks = runNetworkSetupSync(["-setsocksfirewallproxystate", service, "off"])
        let http = runNetworkSetupSync(["-setwebproxystate", service, "off"])
        let https = runNetworkSetupSync(["-setsecurewebproxystate", service, "off"])

        return socks && http && https
    }

    private static func runNetworkSetup(_ arguments: [String]) async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                continuation.resume(returning: runNetworkSetupSync(arguments))
            }
        }
    }

    private static func runNetworkSetupSync(_ arguments: [String]) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        process.arguments = arguments
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice

        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus == 0
        } catch {
            return false
        }
    }
}

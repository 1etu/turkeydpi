import SwiftUI

struct CreateContainerSheet: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss
    
    @State private var name = ""
    @State private var listenAddress = "127.0.0.1"
    @State private var listenPort = 8844
    @State private var preset: ISPPreset = .aggressive
    @State private var autoStart = false
    @State private var enableSystemProxy = true
    
    private var isValid: Bool {
        !name.isEmpty && listenPort > 0 && listenPort < 65536
    }
    
    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section {
                    TextField("Name", text: $name, prompt: Text("My Proxy"))
                }
                
                Section("Network") {
                    TextField("Address", text: $listenAddress)
                    TextField("Port", value: $listenPort, format: .number)
                }
                
                Section("Profile") {
                    Picker("ISP Preset", selection: $preset) {
                        ForEach(ISPPreset.allCases) { p in
                            Text(p.displayName).tag(p)
                        }
                    }
                    
                    Text(preset.description)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                
                Section {
                    Toggle("Start automatically", isOn: $autoStart)
                    Toggle("Set as system proxy", isOn: $enableSystemProxy)
                }
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
            
            Divider()
            
            HStack {
                Button("Cancel") {
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)
                
                Spacer()
                
                Button("Create") {
                    createContainer()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!isValid)
                .buttonStyle(.borderedProminent)
            }
            .padding(16)
        }
        .frame(width: 340, height: 380)
    }
    
    private func createContainer() {
        let config = ContainerConfig(
            name: name,
            listenAddress: listenAddress,
            listenPort: listenPort,
            preset: preset,
            autoStart: autoStart,
            enableSystemProxy: enableSystemProxy
        )
        appState.createContainer(config: config)
        dismiss()
    }
}

#Preview {
    CreateContainerSheet()
        .environmentObject(AppState())
}

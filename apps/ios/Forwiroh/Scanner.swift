import SwiftUI
import AVFoundation

struct ScannerView: UIViewControllerRepresentable {
    let result: (Result<String, Error>) -> Void
    func makeUIViewController(context: Context) -> ScannerController { ScannerController(result: result) }
    func updateUIViewController(_ controller: ScannerController, context: Context) {}
}
/// Session configuration and start/stop calls all run on the camera queue.
private final class CameraCapture: @unchecked Sendable {
    let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "forwiroh.camera")

    func start(delegate: ScannerController, failure: @escaping @MainActor @Sendable (Error) -> Void) {
        queue.async { [self] in
            do {
                guard let device = AVCaptureDevice.default(for: .video) else { throw ForwirohError(message: "No camera available. Paste the invitation instead.") }
                let input = try AVCaptureDeviceInput(device: device)
                let output = AVCaptureMetadataOutput()
                guard session.canAddInput(input), session.canAddOutput(output) else { throw ForwirohError(message: "Cannot start QR scanner") }
                session.addInput(input); session.addOutput(output)
                output.setMetadataObjectsDelegate(delegate, queue: .main)
                output.metadataObjectTypes = [.qr]
                session.startRunning()
            } catch { Task { @MainActor in failure(error) } }
        }
    }
    func stop() { queue.async { [self] in session.stopRunning() } }
}
final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let camera = CameraCapture()
    private var preview: AVCaptureVideoPreviewLayer?
    private var delivered = false
    private let result: (Result<String, Error>) -> Void
    init(result: @escaping (Result<String, Error>) -> Void) { self.result = result; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override func viewDidLoad() {
        super.viewDidLoad()
        let preview = AVCaptureVideoPreviewLayer(session: camera.session)
        preview.videoGravity = .resizeAspectFill
        view.layer.addSublayer(preview); self.preview = preview
        AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
            Task { @MainActor [weak self] in
                guard let self else { return }
                guard granted else { self.finish(.failure(ForwirohError(message: "Camera access denied. Paste an invitation or enable Camera in Settings."))); return }
                self.camera.start(delegate: self) { [weak self] error in self?.finish(.failure(error)) }
            }
        }
    }
    override func viewDidLayoutSubviews() { super.viewDidLayoutSubviews(); preview?.frame = view.bounds }
    override func viewDidDisappear(_ animated: Bool) { super.viewDidDisappear(animated); camera.stop() }
    nonisolated func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard let object = objects.first as? AVMetadataMachineReadableCodeObject, let text = object.stringValue,
              text.hasPrefix("forwiroh://pair?data=") else { return }
        Task { @MainActor [weak self] in self?.finish(.success(text)) }
    }
    private func finish(_ value: Result<String, Error>) {
        guard !delivered else { return }; delivered = true
        camera.stop(); result(value)
    }
}

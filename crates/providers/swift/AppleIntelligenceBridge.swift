// C-ABI bridge from Rust into Apple's FoundationModels framework.
//
// Three symbols are exported, all annotated `@_cdecl` so the static
// library exposes a stable C name without Swift mangling. Async Swift
// APIs are wrapped with a DispatchSemaphore so each call blocks until
// the on-device model returns — the Rust side calls us from
// `tokio::task::spawn_blocking`, never from the async runtime thread.
//
// Memory contract: any `char*` we return is `strdup`-allocated on the
// Swift side and must be freed by the caller via `apple_intel_free`.

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

// Reason codes returned by `apple_intel_availability`. Mirrored verbatim
// in `apple_intelligence.rs::Availability::from_code` — keep both sides
// in sync.
private let kAvailable: Int32 = 0
private let kDeviceNotEligible: Int32 = 1
private let kAppleIntelligenceNotEnabled: Int32 = 2
private let kModelNotReady: Int32 = 3
private let kFrameworkUnavailable: Int32 = 4
private let kOtherUnavailable: Int32 = 5

// Error codes returned by `apple_intel_complete`. Anything non-zero
// means `out_text` was not written.
private let kOk: Int32 = 0
private let kErrUnavailable: Int32 = -1
private let kErrSafety: Int32 = -2
private let kErrInvalidInput: Int32 = -3
private let kErrInternal: Int32 = -4

@_cdecl("apple_intel_availability")
public func apple_intel_availability() -> Int32 {
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        switch SystemLanguageModel.default.availability {
        case .available:
            return kAvailable
        case .unavailable(let reason):
            switch reason {
            case .deviceNotEligible:
                return kDeviceNotEligible
            case .appleIntelligenceNotEnabled:
                return kAppleIntelligenceNotEnabled
            case .modelNotReady:
                return kModelNotReady
            @unknown default:
                return kOtherUnavailable
            }
        @unknown default:
            return kOtherUnavailable
        }
    } else {
        return kFrameworkUnavailable
    }
    #else
    return kFrameworkUnavailable
    #endif
}

@_cdecl("apple_intel_complete")
public func apple_intel_complete(
    _ promptPtr: UnsafePointer<CChar>?,
    _ promptLen: Int,
    _ maxTokens: UInt32,
    _ temperature: Float,
    _ outText: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
    _ outLen: UnsafeMutablePointer<Int>?
) -> Int32 {
    guard let promptPtr = promptPtr, let outText = outText, let outLen = outLen else {
        return kErrInvalidInput
    }

    // Defensive: callers always pass a NUL-terminated buffer, but trust
    // the length field over the terminator since the prompt may contain
    // arbitrary bytes after de-identification.
    let promptData = Data(bytes: promptPtr, count: promptLen)
    guard let prompt = String(data: promptData, encoding: .utf8) else {
        return kErrInvalidInput
    }

    #if canImport(FoundationModels)
    guard #available(macOS 26.0, *) else { return kErrUnavailable }
    guard case .available = SystemLanguageModel.default.availability else {
        return kErrUnavailable
    }

    let sema = DispatchSemaphore(value: 0)
    var resultText: String?
    var errorCode: Int32 = kErrInternal

    Task {
        defer { sema.signal() }
        do {
            let session = LanguageModelSession()
            let options = GenerationOptions(
                temperature: Double(temperature),
                maximumResponseTokens: Int(maxTokens)
            )
            let response = try await session.respond(to: prompt, options: options)
            resultText = response.content
            errorCode = kOk
        } catch let error as LanguageModelSession.GenerationError {
            if case .guardrailViolation = error {
                errorCode = kErrSafety
            } else {
                errorCode = kErrInternal
            }
        } catch {
            errorCode = kErrInternal
        }
    }
    sema.wait()

    if errorCode == kOk, let text = resultText {
        let utf8 = Array(text.utf8)
        // strdup-equivalent: malloc + memcpy + NUL terminator, so the
        // Rust side can `libc::free` (which is exactly what we expose
        // via `apple_intel_free`).
        let buf = UnsafeMutablePointer<CChar>.allocate(capacity: utf8.count + 1)
        utf8.withUnsafeBufferPointer { src in
            buf.withMemoryRebound(to: UInt8.self, capacity: utf8.count) { dst in
                dst.initialize(from: src.baseAddress!, count: utf8.count)
            }
        }
        buf[utf8.count] = 0
        outText.pointee = buf
        outLen.pointee = utf8.count
        return kOk
    }
    return errorCode
    #else
    return kErrUnavailable
    #endif
}

@_cdecl("apple_intel_free")
public func apple_intel_free(_ ptr: UnsafeMutablePointer<CChar>?) {
    guard let ptr = ptr else { return }
    ptr.deallocate()
}

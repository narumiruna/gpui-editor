import CoreGraphics
import Darwin

let windowServer = CGSessionCopyCurrentDictionary() != nil
let input = CGPreflightPostEventAccess()
let capture = CGPreflightScreenCaptureAccess()

print("WindowServer session: \(windowServer)")
print("Synthetic input: \(input)")
print("Screen capture: \(capture)")

// A WindowServer session does not establish that the desktop is safe to use.
exit(windowServer && input && capture ? EXIT_SUCCESS : EXIT_FAILURE)

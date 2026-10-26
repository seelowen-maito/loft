// Runs an evaluation set against Apple's on-device model on this Mac, using the app's own job code
// (Loft/Core/AI/OnDeviceJobs.swift). Usage: Evaluations/run.sh <cases file> [...]
import Foundation
import FoundationModels

struct EvalCase: Decodable {
    let input: String
    var context: String?
    let expected: String
}

struct EvalFile: Decodable {
    let job: String
    /// The run fails below this share of correct answers (0…1).
    let minPassRate: Double
    let cases: [EvalCase]
}

@main
struct Runner {
    static func main() async {
        guard case .available = SystemLanguageModel.default.availability else {
            print("The on-device model isn't available on this Mac: \(SystemLanguageModel.default.availability)")
            exit(2)
        }
        var failedFiles = 0
        for path in CommandLine.arguments.dropFirst() {
            do {
                let file = try JSONDecoder().decode(EvalFile.self, from: Data(contentsOf: URL(fileURLWithPath: path)))
                guard let job = OnDeviceJobs.registry[file.job] ?? Pipelines.registry[file.job] else {
                    print("\(path): no job named \(file.job)")
                    failedFiles += 1
                    continue
                }
                var passed = 0
                var times: [Double] = []
                print("== \(file.job) (\(file.cases.count) cases, needs \(Int(file.minPassRate * 100))%)")
                for c in file.cases {
                    let start = Date()
                    let got = (try? await job(c.input, c.context ?? "")) ?? "<error>"
                    times.append(Date().timeIntervalSince(start))
                    if got == c.expected { passed += 1 } else { print("  ✗ \"\(c.input.prefix(60).replacingOccurrences(of: "\n", with: " / "))…\" → \(got), expected \(c.expected)") }
                }
                times.sort()
                let rate = Double(passed) / Double(max(1, file.cases.count))
                let p = { (q: Double) in times.isEmpty ? 0 : times[min(times.count - 1, Int(Double(times.count) * q))] }
                print(String(format: "  %d/%d passed (%.0f%%) · p50 %.2f s · p95 %.2f s", passed, file.cases.count, rate * 100, p(0.5), p(0.95)))
                if rate < file.minPassRate { failedFiles += 1 }
            } catch {
                print("\(path): \(error)")
                failedFiles += 1
            }
        }
        exit(failedFiles == 0 ? 0 : 1)
    }
}

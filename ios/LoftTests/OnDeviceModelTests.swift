import XCTest
@testable import Loft

/// The wrapper's fallbacks, without the model (these run anywhere; the model's quality is measured by ios/Evaluations).
final class OnDeviceModelTests: XCTestCase {
    func testUnavailableFallsBackWithoutCallingTheModel() async {
        let called = Flag()
        let o = await OnDeviceModel.run("test", available: false, fallback: { "fallback" }) {
            await called.set()
            return "model"
        }
        XCTAssertEqual(o.value, "fallback")
        XCTAssertFalse(o.usedModel)
        let wasCalled = await called.value
        XCTAssertFalse(wasCalled)
    }

    func testTheModelsAnswerIsUsedWhenItComesInTime() async {
        let o = await OnDeviceModel.run("test", timeout: .seconds(1), available: true, fallback: { "fallback" }) { "model" }
        XCTAssertEqual(o.value, "model")
        XCTAssertTrue(o.usedModel)
    }

    func testASlowModelFallsBackOnTime() async {
        let start = Date()
        let o = await OnDeviceModel.run("test", timeout: .milliseconds(200), available: true, fallback: { "fallback" }) {
            try await Task.sleep(for: .seconds(5))
            return "model"
        }
        XCTAssertEqual(o.value, "fallback")
        XCTAssertLessThan(Date().timeIntervalSince(start), 1.5, "the voice path never waits for a slow model")
        if case .fallback(_, let why) = o { XCTAssertTrue(why.contains("longer")) } else { XCTFail() }
    }

    func testAnErrorFallsBack() async {
        struct Boom: Error {}
        let o = await OnDeviceModel.run("test", available: true, fallback: { 1 }) { throw Boom() }
        XCTAssertEqual(o.value, 1)
        XCTAssertFalse(o.usedModel)
    }

    func testStatusWording() {
        XCTAssertEqual(OnDeviceModel.Status.available.text, "Available")
        XCTAssertTrue(OnDeviceModel.Status.appleIntelligenceOff.text.contains("Turned off"))
        XCTAssertFalse(OnDeviceModel.Status.deviceNotEligible.isAvailable)
    }
}

private actor Flag {
    private(set) var value = false
    func set() { value = true }
}

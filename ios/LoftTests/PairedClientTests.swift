import XCTest
@testable import Loft

final class PairedClientTests: XCTestCase {
    /// #170: the list as `GET /api/paired-clients` sends it (chrono's dates, with and without fractions).
    func testDecodesTheServersList() throws {
        let json = #"[{"id":"ffe054fe7ae0cb6d","last_used_at":"2026-11-10T16:23:40.426709+00:00","name":"Chrome on Mac","paired_at":"2026-11-03T09:12:00Z","this":true},{"id":"a0fab1377f49a759","last_used_at":"2026-11-10T15:58:00Z","name":"iPhone","paired_at":"2026-11-04T18:40:00Z","this":false}]"#
        let list = try JSONCoding.decoder.decode([PairedClient].self, from: Data(json.utf8))
        XCTAssertEqual(list.map(\.name), ["Chrome on Mac", "iPhone"])
        XCTAssertEqual(list.map(\.this), [true, false])
        XCTAssertEqual(list[0].pairedAt, JSONCoding.parseDate("2026-11-03T09:12:00Z"))
    }
}

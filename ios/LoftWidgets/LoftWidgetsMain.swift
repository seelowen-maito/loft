import AppIntents
import LoftKit
import SwiftUI
import WidgetKit

@main
struct LoftWidgetsMain: WidgetBundle {
    var body: some Widget {
        LoftOverviewWidget()
        NeedsYouWidget()
        AgentWidget()
        DeliveryWidget()
        TalkWidget()
        if #available(iOS 18.0, macOS 26.0, *) { TalkControl() }
    }
}

/// So the system finds LoftKit's intents: the widgets' buttons and settings.
struct LoftWidgetsIntents: AppIntentsPackage {
    static var includedPackages: [any AppIntentsPackage.Type] { [LoftKitIntents.self] }
}

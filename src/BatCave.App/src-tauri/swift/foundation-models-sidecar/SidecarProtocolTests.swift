import Foundation

@main
struct SidecarProtocolTests {
    static func main() throws {
        try decodesStatusRequest()
        try decodesBoundedGenerationRequest()
        try rejectsInvalidRequests()
        rejectsUnofferedOrFreeTextSelections()
        try encodesBoundedResponse()
        print("Foundation Models sidecar protocol tests passed.")
    }

    private static func decodesStatusRequest() throws {
        let request = try decodeRequest(Data(#"{"version":1,"operation":"status"}"#.utf8))
        precondition(request.operation == .status)
        precondition(request.request == nil)
        precondition(request.facts == nil)
    }

    private static func decodesBoundedGenerationRequest() throws {
        let input = Data(
            #"{"version":1,"operation":"generate","request":{"surface":"overview","publication_seq":42,"fact_digest":"abc123","candidate_ids":["cpu_usage","memory_usage"]},"facts":{"cpu_percent":12.5,"healthy":true}}"#.utf8
        )
        let request = try decodeRequest(input)
        precondition(request.operation == .generate)
        precondition(request.request?.publicationSequence == 42)
        precondition(request.request?.factDigest == "abc123")
        let facts = try canonicalFacts(request.facts!)
        precondition(facts == #"{"cpu_percent":12.5,"healthy":true}"#)
    }

    private static func rejectsInvalidRequests() throws {
        for input in [
            #"{"version":2,"operation":"status"}"#,
            #"{"version":1,"operation":"status","facts":{}}"#,
            #"{"version":1,"operation":"generate"}"#,
        ] {
            do {
                _ = try decodeRequest(Data(input.utf8))
                preconditionFailure("invalid request was accepted: \(input)")
            } catch {
                // Expected.
            }
        }

        let emptyDigest = Data(
            #"{"version":1,"operation":"generate","request":{"surface":"overview","publication_seq":1,"fact_digest":"","candidate_ids":["cpu_usage","memory_usage"]},"facts":{}}"#.utf8
        )
        do {
            _ = try decodeRequest(emptyDigest)
            preconditionFailure("empty fact digest was accepted")
        } catch SidecarProtocolError.invalidRequest {
            // Expected.
        } catch {
            preconditionFailure("empty fact digest failed for the wrong reason: \(error)")
        }

        let oversizedValidRequest = try JSONSerialization.data(withJSONObject: [
            "version": sidecarProtocolVersion,
            "operation": "status",
            "padding": String(repeating: "x", count: maximumInputBytes),
        ])
        do {
            _ = try decodeRequest(oversizedValidRequest)
            preconditionFailure("oversized request was accepted")
        } catch SidecarProtocolError.requestTooLarge {
            // Expected.
        } catch {
            preconditionFailure("valid oversized request failed for the wrong reason: \(error)")
        }
    }

    private static func rejectsUnofferedOrFreeTextSelections() {
        let offered = ["cpu_usage", "memory_usage"]
        precondition(validateSelection("cpu_usage", offered: offered) == "cpu_usage")
        for output in [
            "Safari is showing heavy CPU pressure right now.",
            "cpu_heavy_pressure", "disk_activity", "cpu_usage. Restart Safari.",
            "{\"explanation_id\":\"cpu_usage\"}",
        ] {
            precondition(validateSelection(output, offered: offered) == nil)
        }
    }

    private static func encodesBoundedResponse() throws {
        let response = SidecarResponse(
            availability: .available,
            result: GenerationResult(
                publicationSequence: 7,
                factDigest: "digest",
                text: "cpu_usage"
            )
        )
        let data = try encodeResponse(response)
        precondition(data.count <= maximumOutputBytes)
        precondition(data.last == 0x0a)
        let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        precondition(json?["availability"] as? String == "available")
        let result = json?["result"] as? [String: Any]
        precondition(result?["provider"] as? String == "apple_foundation")
        precondition(result?["publication_seq"] as? Int == 7)

        let oversizedResponse = SidecarResponse(
            availability: .available,
            result: GenerationResult(
                publicationSequence: 7,
                factDigest: "digest",
                text: String(repeating: "x", count: maximumOutputBytes)
            )
        )
        do {
            _ = try encodeResponse(oversizedResponse)
            preconditionFailure("oversized response was accepted")
        } catch SidecarProtocolError.responseTooLarge {
            // Expected.
        } catch {
            preconditionFailure("oversized response failed for the wrong reason: \(error)")
        }
    }
}

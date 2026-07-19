import Foundation

@main
struct FoundationModelsSidecar {
    static func main() async {
        let response: SidecarResponse
        do {
            let data = try readBoundedRequest()
            response = await handleRequest(try decodeRequest(data))
        } catch {
            response = SidecarResponse(availability: .unsupported, result: nil)
        }

        do {
            try FileHandle.standardOutput.write(contentsOf: encodeResponse(response))
        } catch {
            let fallback = Data(
                "{\"availability\":\"runtime_missing\",\"result\":null,\"version\":1}\n".utf8
            )
            try? FileHandle.standardOutput.write(contentsOf: fallback)
        }
    }
}

private func readBoundedRequest() throws -> Data {
    var request = Data()
    while request.count <= maximumInputBytes {
        let remaining = maximumInputBytes + 1 - request.count
        let chunk = try FileHandle.standardInput.read(upToCount: min(4_096, remaining)) ?? Data()
        if chunk.isEmpty { break }
        if let newline = chunk.firstIndex(of: 0x0a) {
            request.append(contentsOf: chunk[..<newline])
            let trailing = chunk[chunk.index(after: newline)...]
            guard trailing.allSatisfy({ $0 == 0x09 || $0 == 0x0a || $0 == 0x0d || $0 == 0x20 }) else {
                throw SidecarProtocolError.invalidRequest
            }
            break
        }
        request.append(chunk)
    }
    if request.last == 0x0d {
        request.removeLast()
    }
    guard request.count <= maximumInputBytes else {
        throw SidecarProtocolError.requestTooLarge
    }
    return request
}

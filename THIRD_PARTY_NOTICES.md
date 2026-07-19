# Third-party notices

## Foundry Local SDK

BatCave can optionally use `foundry-local-sdk` 1.2.0 on Windows and Linux for local narrative generation. The SDK is distributed under the MIT License. Its native runtime includes Microsoft Foundry Local Core and ONNX Runtime components obtained from their pinned Microsoft NuGet packages during the build.

The Windows SDK publishes `Microsoft.AI.Foundry.Local.Core.dll` without an Authenticode signature. BatCave verifies the exact pinned upstream hash before adding its own distribution signature; that signature identifies the BatCave package publisher and does not claim authorship of the Microsoft component. The ONNX Runtime signatures remain unchanged.

Source and license: <https://github.com/microsoft/Foundry-Local/tree/v1.2.0>

## Qwen2.5 0.5B Instruct

BatCave does not bundle the model. A user can explicitly download the pinned Foundry Local catalog variant `qwen2.5-0.5b-instruct-generic-cpu:4` (catalog size: 822 MB). The model is provided under the Apache License 2.0.

License: <https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/blob/main/LICENSE>

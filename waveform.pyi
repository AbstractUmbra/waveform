from typing import Tuple

class AudioResult:
    ogg_data: bytes
    waveform_base64: str
    duration_seconds: float

def generate(audio: bytes) -> Tuple[str, float]: ...

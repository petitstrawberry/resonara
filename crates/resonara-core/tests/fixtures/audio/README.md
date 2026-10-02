These fixtures contain only generated sine waves (no recordings or copyrighted material).
`source.wav` is 5292 frames at 44100 Hz, stereo PCM16: L = 12000 × sin(2π440i/44100),
R = 4000 × sin(2π880i/44100), rounded to signed integers.

The other fixtures were produced with FFmpeg from that source, using the named
codec: FLAC, AIFF/pcm_s16be, CAF/pcm_s16le, M4A/ALAC, M4A/AAC, ADTS/AAC,
MP3/libmp3lame, OGG/libvorbis. `mono.flac` uses FFmpeg's mono downmix.
For example: `ffmpeg -i source.wav -c:a flac stereo.flac`.
Tests decode these checked-in files without requiring FFmpeg at runtime.

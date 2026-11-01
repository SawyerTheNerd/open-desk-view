package com.enzonic.opendeskview

import android.view.SurfaceHolder

/**
 * Where a session's video goes: decodes framebuffer messages of one encoding and shows them on
 * the current surface. [submit] is called from the network thread, [setSurface] from the UI
 * thread, and [release] from either; implementations do their own locking.
 * Adapters: [H264Sink] (hardware MediaCodec) and [JpegSink] (BitmapFactory + Canvas).
 */
interface VideoSink {
    /** The surface to show video on; null while the app is in the background. */
    fun setSurface(holder: SurfaceHolder?)

    /**
     * Take one framebuffer message. Returns true when a whole frame was accepted, i.e. the
     * server should be sent a flow-control ack; withholding it tells the server we stalled.
     * @throws java.io.IOException if video can't recover on this connection, so the session
     *   reconnects for a fresh start, or [SessionEnded] if it can't work at all.
     */
    fun submit(fb: Protocol.Framebuffer, payload: ByteArray): Boolean

    /** Free decoders and threads. Later calls to [submit] accept nothing. */
    fun release()
}

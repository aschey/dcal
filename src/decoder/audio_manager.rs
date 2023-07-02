use super::DecoderError;
use super::{channel_buffer::ChannelBuffer, vec_ext::VecExt, Decoder};
use dasp::sample::Sample as DaspSample;
use rubato::{FftFixedInOut, Resampler};
use symphonia::core::conv::ConvertibleSample;
use symphonia::core::sample::Sample;

struct ResampleDecoderInner<T: Sample + DaspSample> {
    written: usize,
    in_buf: ChannelBuffer<T>,
    resampler: FftFixedInOut<T>,
    resampler_buf: Vec<Vec<T>>,
    out_buf: Vec<T>,
}

impl<T: Sample + DaspSample + ConvertibleSample + rubato::Sample> ResampleDecoderInner<T> {
    fn next(&mut self, decoder: &mut Decoder<T>) -> Result<Option<&[T]>, DecoderError> {
        let mut cur_frame = decoder.current();

        while !self.in_buf.is_full() {
            self.written += self.in_buf.fill_from_slice(&cur_frame[self.written..]);

            if self.written == cur_frame.len() {
                match decoder.next()? {
                    Some(next) => {
                        cur_frame = next;
                        self.written = 0;
                    }
                    None => {
                        return Ok(None);
                    }
                }
            }
        }

        self.resampler
            .process_into_buffer(self.in_buf.inner(), &mut self.resampler_buf, None)
            .expect("number of frames was not correctly calculated");
        self.in_buf.reset();

        self.out_buf.fill_from_deinterleaved(&self.resampler_buf);
        Ok(Some(&self.out_buf))
    }

    fn current(&self) -> &[T] {
        &self.out_buf
    }

    fn flush(&mut self) -> &[T] {
        if self.in_buf.position() > 0 {
            self.in_buf.silence_remainder();
            self.resampler
                .process_into_buffer(self.in_buf.inner(), &mut self.resampler_buf, None)
                .expect("number of frames was not correctly calculated");
            self.in_buf.reset();
            &self.out_buf
        } else {
            &[]
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum ResampledDecoderImpl<T: Sample + DaspSample> {
    Resampled(ResampleDecoderInner<T>),
    NotResampled,
}

pub struct ResampledDecoder<T: Sample + DaspSample> {
    decoder_inner: ResampledDecoderImpl<T>,
    in_sample_rate: usize,
    out_sample_rate: usize,
    channels: usize,
}

impl<T: Sample + DaspSample + ConvertibleSample + rubato::Sample> ResampledDecoder<T> {
    pub fn new(out_sample_rate: usize, channels: usize) -> Self {
        Self {
            decoder_inner: ResampledDecoderImpl::NotResampled,
            in_sample_rate: out_sample_rate,
            out_sample_rate,
            channels,
        }
    }

    pub fn initialize(&mut self, decoder: &mut Decoder<T>) {
        let current_in_rate = self.in_sample_rate;
        self.in_sample_rate = decoder.sample_rate();
        match &mut self.decoder_inner {
            ResampledDecoderImpl::NotResampled => {
                self.initialize_resampler(decoder);
            }
            ResampledDecoderImpl::Resampled(inner) => {
                if self.in_sample_rate != self.out_sample_rate
                    && self.in_sample_rate == current_in_rate
                {
                    inner.written = 0;
                } else if self.in_sample_rate == self.out_sample_rate {
                    self.decoder_inner = ResampledDecoderImpl::NotResampled;
                } else {
                    self.initialize_resampler(decoder);
                }
            }
        }
    }

    fn initialize_resampler(&mut self, decoder: &mut Decoder<T>) {
        let resampler = FftFixedInOut::<T>::new(
            self.in_sample_rate,
            self.out_sample_rate,
            1024,
            self.channels,
        )
        .expect("failed to create resampler");
        let resampler_buf = resampler.input_buffer_allocate();
        let n_frames = resampler.input_frames_next();

        let resampler = ResampledDecoderImpl::Resampled(ResampleDecoderInner {
            // decoder,
            written: 0,
            resampler_buf,
            out_buf: Vec::with_capacity(n_frames * self.channels),
            in_buf: ChannelBuffer::new(n_frames, self.channels),
            resampler,
        });
        self.decoder_inner = resampler;
        self.decode_next_frame(decoder).unwrap();
    }

    pub fn in_sample_rate(&self) -> usize {
        self.in_sample_rate
    }

    pub fn out_sample_rate(&self) -> usize {
        self.out_sample_rate
    }

    pub fn current<'a>(&'a self, decoder: &'a Decoder<T>) -> &[T] {
        match &self.decoder_inner {
            ResampledDecoderImpl::Resampled(decoder_inner) => decoder_inner.current(),
            ResampledDecoderImpl::NotResampled => decoder.current(),
        }
    }

    pub fn flush(&mut self) -> &[T] {
        match &mut self.decoder_inner {
            ResampledDecoderImpl::Resampled(decoder) => decoder.flush(),
            ResampledDecoderImpl::NotResampled => &[],
        }
    }

    pub fn decode_next_frame<'a>(
        &'a mut self,
        decoder: &'a mut Decoder<T>,
    ) -> Result<Option<&[T]>, DecoderError> {
        match &mut self.decoder_inner {
            ResampledDecoderImpl::Resampled(decoder_inner) => decoder_inner.next(decoder),
            ResampledDecoderImpl::NotResampled => decoder.next(),
        }
    }
}

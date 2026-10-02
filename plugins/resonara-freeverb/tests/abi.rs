use clap_sys::ext::{audio_ports::clap_audio_port_info, params::clap_param_info};
use std::mem::{align_of, offset_of, size_of};

#[test]
fn abi_matches_registry_bindings() {
    macro_rules! same {
        ($module:ident::$ty:ident, $($field:ident),+ $(,)?) => {{
            type Local = clap_sys::$module::$ty;
            type Original = clap_sys_upstream::$module::$ty;
            assert_eq!(size_of::<Local>(), size_of::<Original>());
            assert_eq!(align_of::<Local>(), align_of::<Original>());
            $(assert_eq!(offset_of!(Local, $field), offset_of!(Original, $field));)+
        }};
    }
    same!(
        entry::clap_plugin_entry,
        clap_version,
        init,
        deinit,
        get_factory
    );
    same!(
        plugin::clap_plugin_descriptor,
        clap_version,
        id,
        name,
        vendor,
        url,
        manual_url,
        support_url,
        version,
        description,
        features
    );
    same!(
        plugin::clap_plugin,
        desc,
        plugin_data,
        init,
        destroy,
        activate,
        deactivate,
        start_processing,
        stop_processing,
        reset,
        process,
        get_extension,
        on_main_thread
    );
    same!(
        host::clap_host,
        clap_version,
        host_data,
        name,
        vendor,
        url,
        version,
        get_extension,
        request_restart,
        request_process,
        request_callback
    );
    same!(
        process::clap_process,
        steady_time,
        frames_count,
        transport,
        audio_inputs,
        audio_outputs,
        audio_inputs_count,
        audio_outputs_count,
        in_events,
        out_events
    );
    same!(
        audio_buffer::clap_audio_buffer,
        data32,
        data64,
        channel_count,
        latency,
        constant_mask
    );
    same!(
        events::clap_event_param_value,
        header,
        param_id,
        cookie,
        note_id,
        port_index,
        channel,
        key,
        value
    );
    same!(events::clap_input_events, ctx, size, get);
    same!(events::clap_output_events, ctx, try_push);
    same!(stream::clap_istream, ctx, read);
    same!(stream::clap_ostream, ctx, write);
    assert_eq!(
        size_of::<clap_param_info>(),
        size_of::<clap_sys_upstream::ext::params::clap_param_info>()
    );
    assert_eq!(
        size_of::<clap_audio_port_info>(),
        size_of::<clap_sys_upstream::ext::audio_ports::clap_audio_port_info>()
    );
}

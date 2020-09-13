use super::*;
use crate::*;
use alloc::string::String;
use alloc::vec::Vec;
use bytes::{Buf, Bytes};

/// Connection packet initiated by the client
#[derive(Debug, Clone, PartialEq)]
pub struct Connect {
    /// Mqtt protocol version
    pub protocol: Protocol,
    /// Mqtt keep alive time
    pub keep_alive: u16,
    /// Properties
    pub properties: Option<ConnectProperties>,
    /// Client Id
    pub client_id: String,
    /// Clean session. Asks the broker to clear previous state
    pub clean_session: bool,
    /// Will that broker needs to publish when the client disconnects
    pub last_will: Option<LastWill>,
    /// Login credentials
    pub login: Option<Login>,
}

impl Connect {
    pub(crate) fn assemble(fixed_header: FixedHeader, mut bytes: Bytes) -> Result<Connect, Error> {
        let variable_header_index = fixed_header.fixed_len;
        bytes.advance(variable_header_index);

        // Variable header
        let protocol_name = read_mqtt_string(&mut bytes)?;
        let protocol_level = bytes.get_u8();
        if protocol_name != "MQTT" {
            return Err(Error::InvalidProtocol);
        }

        let protocol = match protocol_level {
            5 => Protocol::MQTT(5),
            num => return Err(Error::InvalidProtocolLevel(num)),
        };

        let connect_flags = bytes.get_u8();
        let clean_session = (connect_flags & 0b10) != 0;
        let keep_alive = bytes.get_u16();

        // Properties in variable header
        let properties = ConnectProperties::extract(&mut bytes)?;

        let client_id = read_mqtt_string(&mut bytes)?;
        let last_will = LastWill::extract(connect_flags, &mut bytes)?;
        let login = Login::extract(connect_flags, &mut bytes)?;

        let connect = Connect {
            protocol,
            keep_alive,
            properties,
            client_id,
            clean_session,
            last_will,
            login,
        };

        Ok(connect)
    }

    pub fn new<S: Into<String>>(id: S) -> Connect {
        Connect {
            protocol: Protocol::MQTT(4),
            keep_alive: 10,
            properties: None,
            client_id: id.into(),
            clean_session: true,
            last_will: None,
            login: None,
        }
    }

    pub fn set_login<S: Into<String>>(&mut self, u: S, p: S) -> &mut Connect {
        let login = Login {
            username: u.into(),
            password: p.into(),
        };

        self.login = Some(login);
        self
    }

    pub fn len(&self) -> usize {
        let mut len = 2 + "MQTT".len() // protocol name
                              + 1            // protocol version
                              + 1            // connect flags
                              + 2; // keep alive

        if let Some(properties) = &self.properties {
            let properties_len = properties.len();
            let properties_len_len = remaining_len_len(properties_len);
            len += properties_len_len + properties_len;
        }

        len += 2 + self.client_id.len();

        // last will len
        if let Some(last_will) = &self.last_will {
            len += last_will.len();
        }

        // username and password len
        if let Some(login) = &self.login {
            len += login.len();
        }

        len
    }

    pub fn write(&self, buffer: &mut BytesMut) -> Result<usize, Error> {
        let len = self.len();
        buffer.reserve(len);
        buffer.put_u8(0b0001_0000);
        let count = write_remaining_length(buffer, len)?;
        write_mqtt_string(buffer, "MQTT");
        buffer.put_u8(0x05);
        let flags_index = 1 + count + 2 + 4 + 1;

        let mut connect_flags = 0;
        if self.clean_session {
            connect_flags |= 0x02;
        }

        buffer.put_u8(connect_flags);
        buffer.put_u16(self.keep_alive);
        if let Some(properties) = &self.properties {
            properties.write(buffer)?;
        }

        write_mqtt_string(buffer, &self.client_id);

        if let Some(last_will) = &self.last_will {
            connect_flags |= last_will.write(buffer)?;
        }

        if let Some(login) = &self.login {
            connect_flags |= login.write(buffer);
        }

        // update connect flags
        buffer[flags_index] = connect_flags;
        Ok(len)
    }
}

/// LastWill that broker forwards on behalf of the client
#[derive(Debug, Clone, PartialEq)]
pub struct LastWill {
    pub topic: String,
    pub message: Bytes,
    pub qos: QoS,
    pub retain: bool,
    pub properties: Option<WillProperties>,
}

impl LastWill {
    pub fn new(
        topic: impl Into<String>,
        payload: impl Into<Vec<u8>>,
        qos: QoS,
        retain: bool,
    ) -> LastWill {
        LastWill {
            topic: topic.into(),
            message: Bytes::from(payload.into()),
            qos,
            retain,
            properties: None,
        }
    }

    fn extract(connect_flags: u8, mut bytes: &mut Bytes) -> Result<Option<LastWill>, Error> {
        let last_will = match connect_flags & 0b100 {
            0 if (connect_flags & 0b0011_1000) != 0 => {
                return Err(Error::IncorrectPacketFormat);
            }
            0 => None,
            _ => {
                let properties = WillProperties::extract(&mut bytes)?;
                let will_topic = read_mqtt_string(&mut bytes)?;
                let will_message = read_mqtt_bytes(&mut bytes)?;
                let will_qos = qos((connect_flags & 0b11000) >> 3)?;
                Some(LastWill {
                    topic: will_topic,
                    message: will_message,
                    qos: will_qos,
                    retain: (connect_flags & 0b0010_0000) != 0,
                    properties,
                })
            }
        };

        Ok(last_will)
    }

    fn len(&self) -> usize {
        let mut len = 0;
        if let Some(properties) = &self.properties {
            let properties_len = properties.len();
            let properties_len_len = remaining_len_len(properties_len);
            len += properties_len_len + properties_len;
        };

        len += 2 + self.topic.len() + 2 + self.message.len();
        len
    }

    fn write(&self, buffer: &mut BytesMut) -> Result<u8, Error> {
        let mut connect_flags = 0;

        connect_flags |= 0x04 | (self.qos as u8) << 3;
        if self.retain {
            connect_flags |= 0x20;
        }

        if let Some(properties) = &self.properties {
            properties.write(buffer)?;
        }

        write_mqtt_string(buffer, &self.topic);
        write_mqtt_bytes(buffer, &self.message);
        Ok(connect_flags)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WillProperties {
    delay_interval: Option<u32>,
    payload_format_indicator: Option<u8>,
    message_expiry_interval: Option<u32>,
    content_type: Option<String>,
    response_topic: Option<String>,
    correlation_data: Option<Bytes>,
    user_properties: Vec<(String, String)>,
}

impl WillProperties {
    fn len(&self) -> usize {
        let mut len = 0;

        if self.delay_interval.is_some() {
            len += 1 + 4;
        }

        if self.payload_format_indicator.is_some() {
            len += 1 + 1;
        }

        if self.message_expiry_interval.is_some() {
            len += 1 + 4;
        }

        if let Some(typ) = &self.content_type {
            len += 1 + 2 + typ.len()
        }

        if let Some(topic) = &self.response_topic {
            len += 1 + 2 + topic.len()
        }

        if let Some(data) = &self.correlation_data {
            len += 1 + 2 + data.len()
        }

        for (key, value) in self.user_properties.iter() {
            len += 1 + 2 + key.len() + 2 + value.len();
        }

        len
    }

    fn extract(mut bytes: &mut Bytes) -> Result<Option<WillProperties>, Error> {
        let mut delay_interval = None;
        let mut payload_format_indicator = None;
        let mut message_expiry_interval = None;
        let mut content_type = None;
        let mut response_topic = None;
        let mut correlation_data = None;
        let mut user_properties = Vec::new();

        let (properties_len_len, properties_len) = length(bytes.iter())?;
        bytes.advance(properties_len_len);
        if properties_len == 0 {
            return Ok(None);
        }

        let mut cursor = 0;
        // read until cursor reaches property length. properties_len = 0 will skip this loop
        while cursor < properties_len {
            let prop = bytes.get_u8();
            cursor += 1;

            match property(prop)? {
                PropertyType::WillDelayInterval => {
                    delay_interval = Some(bytes.get_u32());
                    cursor += 4;
                }
                PropertyType::PayloadFormatIndicator => {
                    payload_format_indicator = Some(bytes.get_u8());
                    cursor += 1;
                }
                PropertyType::MessageExpiryInterval => {
                    message_expiry_interval = Some(bytes.get_u32());
                    cursor += 4;
                }
                PropertyType::ContentType => {
                    let typ = read_mqtt_string(&mut bytes)?;
                    cursor += 2 + typ.len();
                    content_type = Some(typ);
                }
                PropertyType::ResponseTopic => {
                    let topic = read_mqtt_string(&mut bytes)?;
                    cursor += 2 + topic.len();
                    response_topic = Some(topic);
                }
                PropertyType::CorrelationData => {
                    let data = read_mqtt_bytes(&mut bytes)?;
                    cursor += 2 + data.len();
                    correlation_data = Some(data);
                }
                PropertyType::UserProperty => {
                    let key = read_mqtt_string(&mut bytes)?;
                    let value = read_mqtt_string(&mut bytes)?;
                    cursor += 2 + key.len() + 2 + value.len();
                    user_properties.push((key, value));
                }
                _ => return Err(Error::InvalidPropertyType(prop)),
            }
        }

        Ok(Some(WillProperties {
            delay_interval,
            payload_format_indicator,
            message_expiry_interval,
            content_type,
            response_topic,
            correlation_data,
            user_properties,
        }))
    }

    fn write(&self, buffer: &mut BytesMut) -> Result<(), Error> {
        let len = self.len();
        write_remaining_length(buffer, len)?;

        if let Some(delay_interval) = self.delay_interval {
            buffer.put_u8(PropertyType::WillDelayInterval as u8);
            buffer.put_u32(delay_interval);
        }

        if let Some(payload_format_indicator) = self.payload_format_indicator {
            buffer.put_u8(PropertyType::PayloadFormatIndicator as u8);
            buffer.put_u8(payload_format_indicator);
        }

        if let Some(message_expiry_interval) = self.message_expiry_interval {
            buffer.put_u8(PropertyType::MessageExpiryInterval as u8);
            buffer.put_u32(message_expiry_interval);
        }

        if let Some(typ) = &self.content_type {
            buffer.put_u8(PropertyType::ContentType as u8);
            write_mqtt_string(buffer, typ);
        }

        if let Some(topic) = &self.response_topic {
            buffer.put_u8(PropertyType::ResponseTopic as u8);
            write_mqtt_string(buffer, topic);
        }

        if let Some(data) = &self.correlation_data {
            buffer.put_u8(PropertyType::CorrelationData as u8);
            write_mqtt_bytes(buffer, data);
        }

        for (key, value) in self.user_properties.iter() {
            buffer.put_u8(PropertyType::UserProperty as u8);
            write_mqtt_string(buffer, key);
            write_mqtt_string(buffer, value);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    username: String,
    password: String,
}

impl Login {
    fn extract(connect_flags: u8, mut bytes: &mut Bytes) -> Result<Option<Login>, Error> {
        let username = match connect_flags & 0b1000_0000 {
            0 => String::new(),
            _ => read_mqtt_string(&mut bytes)?,
        };

        let password = match connect_flags & 0b0100_0000 {
            0 => String::new(),
            _ => read_mqtt_string(&mut bytes)?,
        };

        if username.is_empty() && password.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Login { username, password }))
        }
    }

    fn len(&self) -> usize {
        let mut len = 0;

        if !self.username.is_empty() {
            len += 2 + self.username.len();
        }

        if !self.password.is_empty() {
            len += 2 + self.password.len();
        }

        len
    }

    fn write(&self, buffer: &mut BytesMut) -> u8 {
        let mut connect_flags = 0;
        if !self.username.is_empty() {
            connect_flags |= 0x80;
            write_mqtt_string(buffer, &self.username);
        }

        if !self.password.is_empty() {
            connect_flags |= 0x40;
            write_mqtt_string(buffer, &self.password);
        }

        connect_flags
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectProperties {
    /// Expiry interval property after loosing connection
    pub session_expiry_interval: Option<u32>,
    /// Maximum simultaneous packets
    pub receive_maximum: Option<u16>,
    /// Maximum packet size
    pub max_packet_size: Option<u32>,
    /// Maximum mapping integer for a topic
    pub topic_alias_max: Option<u16>,
    pub request_response_info: Option<u8>,
    pub request_problem_info: Option<u8>,
    /// List of user properties
    pub user_properties: Vec<(String, String)>,
    /// Method of authentication
    pub authentication_method: Option<String>,
    /// Authentication data
    pub authentication_data: Option<Bytes>,
}

impl ConnectProperties {
    fn _new() -> ConnectProperties {
        ConnectProperties {
            session_expiry_interval: None,
            receive_maximum: None,
            max_packet_size: None,
            topic_alias_max: None,
            request_response_info: None,
            request_problem_info: None,
            user_properties: Vec::new(),
            authentication_method: None,
            authentication_data: None,
        }
    }

    fn extract(mut bytes: &mut Bytes) -> Result<Option<ConnectProperties>, Error> {
        let mut session_expiry_interval = None;
        let mut receive_maximum = None;
        let mut max_packet_size = None;
        let mut topic_alias_max = None;
        let mut request_response_info = None;
        let mut request_problem_info = None;
        let mut user_properties = Vec::new();
        let mut authentication_method = None;
        let mut authentication_data = None;

        let (properties_len_len, properties_len) = length(bytes.iter())?;
        bytes.advance(properties_len_len);
        if properties_len == 0 {
            return Ok(None);
        }

        let mut cursor = 0;
        // read until cursor reaches property length. properties_len = 0 will skip this loop
        while cursor < properties_len {
            let prop = bytes.get_u8();
            cursor += 1;
            match property(prop)? {
                PropertyType::SessionExpiryInterval => {
                    session_expiry_interval = Some(bytes.get_u32());
                    cursor += 4;
                }
                PropertyType::ReceiveMaximum => {
                    receive_maximum = Some(bytes.get_u16());
                    cursor += 2;
                }
                PropertyType::MaximumPacketSize => {
                    max_packet_size = Some(bytes.get_u32());
                    cursor += 4;
                }
                PropertyType::TopicAliasMaximum => {
                    topic_alias_max = Some(bytes.get_u16());
                    cursor += 2;
                }
                PropertyType::RequestResponseInformation => {
                    request_response_info = Some(bytes.get_u8());
                    cursor += 1;
                }
                PropertyType::RequestProblemInformation => {
                    request_problem_info = Some(bytes.get_u8());
                    cursor += 1;
                }
                PropertyType::UserProperty => {
                    let key = read_mqtt_string(&mut bytes)?;
                    let value = read_mqtt_string(&mut bytes)?;
                    cursor += 2 + key.len() + 2 + value.len();
                    user_properties.push((key, value));
                }
                PropertyType::AuthenticationMethod => {
                    let method = read_mqtt_string(&mut bytes)?;
                    cursor += 2 + method.len();
                    authentication_method = Some(method);
                }
                PropertyType::AuthenticationData => {
                    let data = read_mqtt_bytes(&mut bytes)?;
                    cursor += 2 + data.len();
                    authentication_data = Some(data);
                }
                _ => return Err(Error::InvalidPropertyType(prop)),
            }
        }

        Ok(Some(ConnectProperties {
            session_expiry_interval,
            receive_maximum,
            max_packet_size,
            topic_alias_max,
            request_response_info,
            request_problem_info,
            user_properties,
            authentication_method,
            authentication_data,
        }))
    }

    fn len(&self) -> usize {
        let mut len = 0;

        if self.session_expiry_interval.is_some() {
            len += 1 + 4;
        }

        if self.receive_maximum.is_some() {
            len += 1 + 2;
        }

        if self.max_packet_size.is_some() {
            len += 1 + 4;
        }

        if self.topic_alias_max.is_some() {
            len += 1 + 2;
        }

        if self.request_response_info.is_some() {
            len += 1 + 1;
        }

        if self.request_problem_info.is_some() {
            len += 1 + 1;
        }

        for (key, value) in self.user_properties.iter() {
            len += 1 + 2 + key.len() + 2 + value.len();
        }

        if let Some(authentication_method) = &self.authentication_method {
            len += 1 + 2 + authentication_method.len();
        }

        if let Some(authentication_data) = &self.authentication_data {
            len += 1 + 2 + authentication_data.len();
        }

        len
    }

    fn write(&self, buffer: &mut BytesMut) -> Result<(), Error> {
        let len = self.len();
        write_remaining_length(buffer, len)?;

        // TODO: This approach to use if let Some() might be bug prone. We might
        // TODO: miss a field or put wrong_u8
        if let Some(session_expiry_interval) = self.session_expiry_interval {
            buffer.put_u8(PropertyType::SessionExpiryInterval as u8);
            buffer.put_u32(session_expiry_interval);
        }

        if let Some(receive_maximum) = self.receive_maximum {
            buffer.put_u8(PropertyType::ReceiveMaximum as u8);
            buffer.put_u16(receive_maximum);
        }

        if let Some(max_packet_size) = self.max_packet_size {
            buffer.put_u8(PropertyType::MaximumPacketSize as u8);
            buffer.put_u32(max_packet_size);
        }

        if let Some(topic_alias_max) = self.topic_alias_max {
            buffer.put_u8(PropertyType::TopicAliasMaximum as u8);
            buffer.put_u16(topic_alias_max);
        }

        if let Some(request_response_info) = self.request_response_info {
            buffer.put_u8(PropertyType::RequestResponseInformation as u8);
            buffer.put_u8(request_response_info);
        }

        if let Some(request_problem_info) = self.request_problem_info {
            buffer.put_u8(PropertyType::RequestProblemInformation as u8);
            buffer.put_u8(request_problem_info);
        }

        for (key, value) in self.user_properties.iter() {
            buffer.put_u8(PropertyType::UserProperty as u8);
            write_mqtt_string(buffer, key);
            write_mqtt_string(buffer, value);
        }

        if let Some(authentication_method) = &self.authentication_method {
            buffer.put_u8(PropertyType::AuthenticationMethod as u8);
            write_mqtt_string(buffer, authentication_method);
        }

        if let Some(authentication_data) = &self.authentication_data {
            buffer.put_u8(PropertyType::AuthenticationData as u8);
            write_mqtt_bytes(buffer, authentication_data);
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::*;
    use alloc::vec;
    use bytes::{Bytes, BytesMut};
    use pretty_assertions::assert_eq;

    fn sample() -> Connect {
        let connect_properties = ConnectProperties {
            session_expiry_interval: Some(1234),
            receive_maximum: Some(432),
            max_packet_size: Some(100),
            topic_alias_max: Some(456),
            request_response_info: Some(1),
            request_problem_info: Some(1),
            user_properties: vec![("test".to_owned(), "test".to_owned())],
            authentication_method: Some("test".to_owned()),
            authentication_data: Some(Bytes::from(vec![1, 2, 3, 4])),
        };

        let will_properties = WillProperties {
            delay_interval: Some(1234),
            payload_format_indicator: Some(0),
            message_expiry_interval: Some(4321),
            content_type: Some("test".to_owned()),
            response_topic: Some("topic".to_owned()),
            correlation_data: Some(Bytes::from(vec![1, 2, 3, 4])),
            user_properties: vec![("test".to_owned(), "test".to_owned())],
        };

        let will = LastWill {
            topic: "mydevice/status".to_string(),
            message: Bytes::from(vec![b'd', b'e', b'a', b'd']),
            qos: QoS::AtMostOnce,
            retain: false,
            properties: Some(will_properties),
        };

        let login = Login {
            username: "matteo".to_string(),
            password: "collina".to_string(),
        };

        Connect {
            protocol: Protocol::MQTT(5),
            keep_alive: 0,
            properties: Some(connect_properties),
            client_id: "my-device".to_string(),
            clean_session: true,
            last_will: Some(will),
            login: Some(login),
        }
    }

    fn sample_bytes() -> Vec<u8> {
        vec![
            0x10, // packet type
            0x9d, // remaining len
            0x01, // remaining len
            00, 04,   // 4
            0x4d, // M
            0x51, // Q
            0x54, // T
            0x54, // T
            0x05, // Level
            0xc6, // connect flags
            0x00, 0x00, // keep alive
            0x2f, // properties len
            0x11, 0x00, 0x00, 0x04, 0xd2, // session expiry interval
            0x21, 0x01, 0xb0, // receive_maximum
            0x27, 0x00, 0x00, 0x00, 0x64, // max packet size
            0x22, 0x01, 0xc8, // topic_alias_max
            0x19, 0x01, // request_response_info
            0x17, 0x01, // request_problem_info
            0x26, 0x00, 0x04, 0x74, 0x65, 0x73, 0x74, 0x00, 0x04, 0x74, 0x65, 0x73,
            0x74, // user
            0x15, 0x00, 0x04, 0x74, 0x65, 0x73, 0x74, // authentication_method
            0x16, 0x00, 0x04, 0x01, 0x02, 0x03, 0x04, // authentication_data
            0x00, 0x09, 0x6d, 0x79, 0x2d, 0x64, 0x65, 0x76, 0x69, 0x63, 0x65, // client id
            0x2f, // will properties len
            0x18, 0x00, 0x00, 0x04, 0xd2, // will delay interval
            0x01, 0x00, // payload format indicator
            0x02, 0x00, 0x00, 0x10, 0xe1, // message expiry interval
            0x03, 0x00, 0x04, 0x74, 0x65, 0x73, 0x74, // content type
            0x08, 0x00, 0x05, 0x74, 0x6f, 0x70, 0x69, 0x63, // response topic
            0x09, 0x00, 0x04, 0x01, 0x02, 0x03, 0x04, // correlation_data
            0x26, 0x00, 0x04, 0x74, 0x65, 0x73, 0x74, 0x00, 0x04, 0x74, 0x65, 0x73,
            0x74, // will user properties
            0x00, 0x0f, 0x6d, 0x79, 0x64, 0x65, 0x76, 0x69, 0x63, 0x65, 0x2f, 0x73, 0x74, 0x61,
            0x74, 0x75, 0x73, // will topic
            0x00, 0x04, 0x64, 0x65, 0x61, 0x64, // will payload
            0x00, 0x06, 0x6d, 0x61, 0x74, 0x74, 0x65, 0x6f, // username
            0x00, 0x07, 0x63, 0x6f, 0x6c, 0x6c, 0x69, 0x6e, 0x61, // password
        ]
    }

    #[test]
    fn connect_parsing_works_correctlyl() {
        let mut stream = bytes::BytesMut::new();
        let packetstream = &sample_bytes();
        stream.extend_from_slice(&packetstream[..]);
        let packet = mqtt_read(&mut stream, 200).unwrap();
        let packet = match packet {
            Packet::Connect(connect) => connect,
            packet => panic!("Invalid packet = {:?}", packet),
        };

        let connect = sample();
        assert_eq!(packet, connect);
    }

    #[test]
    fn connect_encoding_works_correctly() {
        let connect = sample();
        let mut buf = BytesMut::new();
        connect.write(&mut buf).unwrap();
        assert_eq!(&buf[..], sample_bytes());
    }

    #[test]
    fn missing_properties_are_encoded_correctly() {}
}

package dev.buildlens.analytics.messaging;

import tools.jackson.core.JacksonException;
import tools.jackson.databind.ObjectMapper;
import com.rabbitmq.client.Channel;
import dev.buildlens.analytics.metrics.AnalyticsCoordinator;
import java.io.IOException;
import java.util.UUID;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.amqp.core.Message;
import org.springframework.amqp.rabbit.annotation.RabbitListener;
import org.springframework.dao.DataAccessException;
import org.springframework.stereotype.Component;

@Component
public class AnalyticsEventListener {
    private static final Logger LOG = LoggerFactory.getLogger(AnalyticsEventListener.class);

    private final ObjectMapper objectMapper;
    private final AnalyticsCoordinator coordinator;

    public AnalyticsEventListener(ObjectMapper objectMapper, AnalyticsCoordinator coordinator) {
        this.objectMapper = objectMapper;
        this.coordinator = coordinator;
    }

    @RabbitListener(queues = "${analytics.queue}")
    public void consume(Message message, Channel channel) throws IOException {
        long tag = message.getMessageProperties().getDeliveryTag();
        try {
            EventEnvelope envelope = objectMapper.readValue(message.getBody(), EventEnvelope.class);
            EventPolicy.validate(envelope);
            validateMessageId(message, envelope.id());
            switch (envelope.type()) {
                case "workflow_run.completed" -> coordinator.workflowRunCompleted(
                        envelope.aggregate().id(), envelope.repositoryId(), envelope.occurredAt());
                case "deployment.recorded" ->
                        coordinator.deploymentRecorded(
                                envelope.aggregate().id(), envelope.repositoryId(), envelope.occurredAt());
                default -> throw new FatalEventException("unsupported event type: " + envelope.type());
            }
            channel.basicAck(tag, false);
            LOG.info("processed analytics trigger event_id={} type={}", envelope.id(), envelope.type());
        } catch (JacksonException | FatalEventException | IllegalArgumentException exception) {
            LOG.warn("dead-lettering invalid analytics event: {}", exception.getMessage());
            channel.basicReject(tag, false);
        } catch (DataAccessException exception) {
            LOG.warn("requeueing analytics event after database failure", exception);
            channel.basicNack(tag, false, true);
        } catch (RuntimeException exception) {
            LOG.error("dead-lettering poison analytics event", exception);
            channel.basicReject(tag, false);
        }
    }

    private static void validateMessageId(Message message, UUID envelopeId) {
        String messageId = message.getMessageProperties().getMessageId();
        if (messageId == null || !messageId.equals(envelopeId.toString())) {
            throw new FatalEventException("RabbitMQ message_id does not match envelope id");
        }
    }
}

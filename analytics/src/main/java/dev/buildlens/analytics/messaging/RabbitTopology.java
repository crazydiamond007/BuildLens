package dev.buildlens.analytics.messaging;

import org.springframework.amqp.core.Binding;
import org.springframework.amqp.core.BindingBuilder;
import org.springframework.amqp.core.Queue;
import org.springframework.amqp.core.QueueBuilder;
import org.springframework.amqp.core.TopicExchange;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

@Configuration
public class RabbitTopology {
    public static final String EVENTS_EXCHANGE = "buildlens.events";
    public static final String DEAD_LETTER_EXCHANGE = "buildlens.events.dlx";

    @Bean
    TopicExchange eventsExchange() {
        return new TopicExchange(EVENTS_EXCHANGE, true, false);
    }

    @Bean
    TopicExchange deadLetterExchange() {
        return new TopicExchange(DEAD_LETTER_EXCHANGE, true, false);
    }

    @Bean
    Queue analyticsQueue(@Value("${analytics.queue}") String queueName) {
        return QueueBuilder.durable(queueName)
                .deadLetterExchange(DEAD_LETTER_EXCHANGE)
                .build();
    }

    @Bean
    Binding workflowBinding(Queue analyticsQueue, TopicExchange eventsExchange) {
        return BindingBuilder.bind(analyticsQueue).to(eventsExchange).with("workflow_run.*");
    }

    @Bean
    Binding deploymentBinding(Queue analyticsQueue, TopicExchange eventsExchange) {
        return BindingBuilder.bind(analyticsQueue).to(eventsExchange).with("deployment.*");
    }
}
